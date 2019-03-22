use super::{Open, Sink};
extern crate rodio;
extern crate cpal;
use self::rodio::Source;

use std::time::Duration;
use std::io;
use std::process::exit;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};

pub struct RodioSink {
    rodio_device: rodio::Device,
    send: SyncSender<i16>,
    stopped: Arc<AtomicBool>,
}

fn list_outputs() {
    println!("Default Audio Device:\n  {:?}", rodio::default_output_device().map(|e| e.name()));

    println!("Available Audio Devices:");
    for device in rodio::output_devices() {
        println!("- {}", device.name());
        // Output formats
        if let Ok(fmt) = device.default_output_format() {
            println!("  Default format:\n    {:?}", fmt);
        }
        let mut output_formats = match device.supported_output_formats() {
            Ok(f) => f.peekable(),
            Err(e) => {
                println!("Error: {:?}", e);
                continue;
            },
        };
        if output_formats.peek().is_some() {
            println!("  All formats:");
            for format in output_formats {
                println!("    {:?}", format);
            }
        }
    }
}

impl Open for RodioSink {
    fn open(device: Option<String>) -> RodioSink {
        info!("Using rodio sink");

        let mut rodio_device = rodio::default_output_device().expect("no output device available");
        if device.is_some() {
            let device_name = device.unwrap();

            if device_name == "?".to_string() {
                list_outputs();
                exit(0)
            }
            let mut found = false;
            for d in rodio::output_devices() {
                if d.name() == device_name {
                    rodio_device = d;
                    found = true;
                    break;
                }
            }
            if !found {
                println!("No output sink matching '{}' found.", device_name);
                exit(0)
            }
        }

        let source = RodioSink {
            rodio_device: rodio_device,
            // quick hack so we don't have to use an Option
            send: {
                let (tx, _rx) = sync_channel(0);
                tx
            },
            stopped: Arc::new(AtomicBool::new(true)),
        };

        source
    }
}

impl Sink for RodioSink {
    fn start(&mut self) -> io::Result<()> {
        info!("Start RodioSink");
        //                  100ms = 2 * 4410
        let (tx, rx) = sync_channel(2 * 4096);
        self.send = tx;
        self.stopped = Arc::new(AtomicBool::new(false));
        let source = LibrespotSource {
            recv: rx,
            stopped: self.stopped.clone(),
        };
        rodio::play_raw(&self.rodio_device, source.convert_samples());
        Ok(())
    }

    fn stop(&mut self) -> io::Result<()> {
        info!("Stop RodioSink");
        self.stopped.store(false, Ordering::Relaxed);
        Ok(())
    }

    fn write(&mut self, data: &[i16]) -> io::Result<()> {
        for s in data.iter() {
            let r = self.send.send(*s);
            if r.is_err() {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "Rodio Sink: Reciever disconnected."));
            }
        }
        Ok(())
    }

}


struct LibrespotSource {
    recv: Receiver<i16>,
    stopped: Arc<AtomicBool>,
}

impl Iterator for LibrespotSource {
    type Item = i16;

    #[inline]
    fn next(&mut self) -> Option<i16> {
        match self.stopped.load(Ordering::Relaxed) {
            true => None,
            false => self.recv.try_iter().next()
        }
    }
}

impl rodio::Source for LibrespotSource {
    #[inline]
    fn current_frame_len(&self) -> Option<usize> {
        None
    }

    #[inline]
    fn channels(&self) -> u16 {
        2
    }

    #[inline]
    fn sample_rate(&self) -> u32 {
        44100
    }

    #[inline]
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}