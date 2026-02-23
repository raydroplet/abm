// gui.rs

use crate::engine::{Engine, EngineCommand, FrameData};

use crate::view_egui::ViewEGUI;
use crate::view_macroquad::ViewMacroquad;
pub use crossbeam_channel as crossbeam;
use std::thread;

pub enum ProducerCommand {
    PLAY,
    PAUSE,
    STEP,
}

pub enum Command {
    Producer(ProducerCommand),
    Engine(EngineCommand),
}

impl From<EngineCommand> for Command {
    fn from(cmd: EngineCommand) -> Self {
        Command::Engine(cmd)
    }
}

pub struct Producer {
    returner: crossbeam::Sender<FrameData>,
    receiver: crossbeam::Receiver<FrameData>,
    command_receiver: crossbeam::Receiver<Command>,
    //
    to_tick: bool,
    single_step: bool,
}

impl Producer {
    pub fn new(
        engine_receiver: crossbeam::Receiver<FrameData>,
        engine_returner: crossbeam::Sender<FrameData>,
        command_receiver: crossbeam::Receiver<Command>,
    ) -> Self {
        Self {
            returner: engine_returner,
            receiver: engine_receiver,
            command_receiver: command_receiver,
            to_tick: true,
            single_step: false,
        }
    }

    // Takes ownership of Engine and runs it in a background thread
    pub fn run_thread(mut self, mut engine: Engine) {
        thread::spawn(move || {
            // NOTE: this acts as a while(true) when paused, which may not be ideal
            loop {
                match self.command_receiver.try_recv() {
                    Ok(command) => match command {
                        Command::Producer(producer_command) => {
                            self.handle(producer_command);
                        }
                        Command::Engine(engine_command) => {
                            engine.handle(engine_command);
                        }
                    },
                    Err(_) => {}
                }

                // Inside Producer::run_thread's loop
                if self.to_tick {
                    // Normal play mode: uses the internal while-loop with FIXED_DT
                    engine.tick();
                } else if self.single_step {
                    // Debug step mode: forces exactly one simulation frame
                    engine.tick_once();
                    self.single_step = false;
                }

                match self.receiver.try_recv() {
                    Ok(mut frame) => {
                        // A buffer is available! We can render.
                        engine.render(&mut frame);

                        // Send it to the UI
                        if self.returner.send(frame).is_err() {
                            break; // UI closed
                        }
                    }
                    Err(crossbeam::TryRecvError::Empty) => {
                        // No buffer available yet.
                        // The UI is still drawing the previous frame.
                        // Just loop back and tick again!
                        continue;
                    }
                    Err(crossbeam::TryRecvError::Disconnected) => {
                        break; // UI closed
                    }
                }
            }
        });
    }

    fn handle(&mut self, command: ProducerCommand) {
        match command {
            ProducerCommand::PLAY => {
                self.to_tick = true;
            }
            ProducerCommand::PAUSE => {
                self.to_tick = false;
            }
            ProducerCommand::STEP => {
                self.single_step = true;
            }
        }
    }
}

// Small wrapper for the eframe::App trait
//
// Implements the update() method that queries a frame
// and sends it to eframe to be presented on the window
//
pub struct Presenter {
    receiver: crossbeam::Receiver<FrameData>,
    returner: crossbeam::Sender<FrameData>,
    command_sender: crossbeam::Sender<Command>,
    // the renderer
    view_egui: ViewEGUI,
    view_macroquad: ViewMacroquad,
}

impl Presenter {
    pub fn new(
        frame_receiver: crossbeam::Receiver<FrameData>,
        frame_returner: crossbeam::Sender<FrameData>,
        command_sender: crossbeam::Sender<Command>,
    ) -> Self {
        let (width, height) = (1024, 768);

        let receive_frame = {
            let receiver = frame_receiver.clone();
            let returner = frame_returner.clone();
            move |current_frame: &mut Option<FrameData>| {
                Self::receive_frame(&receiver, &returner , current_frame)
            }
        };

        let return_frame = {
            let returner = frame_returner.clone();
            move |frame: Option<FrameData>| {
                Self::return_frame(&returner, frame);
            }
        };

        Self {
            // thread communication
            receiver: frame_receiver,
            returner: frame_returner,
            command_sender: command_sender.clone(),
            view_egui: ViewEGUI::new(width, height, receive_frame, return_frame, command_sender),
            view_macroquad: ViewMacroquad::new(width as u32, height as u32),
        }
    }

    fn receive_frame(
        receiver: &crossbeam::Receiver<FrameData>,
        returner: &crossbeam::Sender<FrameData>,
        current_frame: &mut Option<FrameData>,
    ) -> Option<FrameData> {
        // gets the newest frame from the engine
        // replaces old one if not consumed
        let mut newest_frame = None;
        while let Ok(frame) = receiver.try_recv() {
            if let Some(skipped_frame) = newest_frame.replace(frame) {
                let _ = returner.send(skipped_frame);
            }
        }

        // cache the frame contents
        // we may wish to draw the previous frame if a new one isn't available
        let mut frame_to_recycle = newest_frame;
        if let Some(new_frame) = frame_to_recycle {
            frame_to_recycle = current_frame.replace(new_frame);
        }

        frame_to_recycle
    }

    fn return_frame(returner: &crossbeam::Sender<FrameData>, frame: Option<FrameData>) {
        // send the old buffer back to the engine
        if let Some(old_buffer) = frame {
            let _ = returner.send(old_buffer);
        }
    }

    pub fn run(self) {
        let (width, height) = self.view_egui.dimensions();

        for _ in 0..2 {
            let _ = self.returner.send(FrameData::new(width, height));
        }

        // let _ = self.view_egui.run();
        self.view_macroquad.run();
    }
}
