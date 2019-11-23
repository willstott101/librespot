use super::{Open, Sink};
extern crate sample;
extern crate cpal;
use std::{io, thread};
use std::process::exit;
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::{Arc};

use self::cpal::traits::{DeviceTrait, EventLoopTrait, HostTrait};
use self::sample::{interpolate, signal, Sample, Signal};

struct ResampleParams {
    last_frame: [i16; 2],
    target_sample_rate: f64,
}

pub struct CpalSink {
    host: cpal::Host,
    device_name: Option<String>,
    event_loop: Arc<cpal::EventLoop>,
    stream_id: Option<cpal::StreamId>,
    send: SyncSender<i16>,
    resample: Option<ResampleParams>,
}

fn list_formats(ref device: &cpal::Device) {
    let default_fmt = match device.default_output_format() {
        Ok(fmt) => cpal::SupportedFormat::from(fmt),
        Err(e) => {
            warn!("Error getting default cpal::Device format: {:?}", e);
            return;
        },
    };

    let mut output_formats = match device.supported_output_formats() {
        Ok(f) => f.peekable(),
        Err(e) => {
            warn!("Error getting supported cpal::Device formats: {:?}", e);
            return;
        },
    };

    if output_formats.peek().is_some() {
        debug!("  Available formats:");
        for format in output_formats {
            let s = format!("{}ch, {:?}, min {:?}, max {:?}", format.channels, format.data_type, format.min_sample_rate, format.max_sample_rate);
            if format == default_fmt {
                debug!("    (default) {}", s);
            } else {
                debug!("    {:?}", format);
            }
        }
    }
}

fn list_outputs(host: &cpal::Host) {
    let default_device = host.default_output_device().unwrap();
    let default_name = default_device.name().expect("Failed to access cpal::Device name");
    println!("Default Audio Device:\n  {}", default_name);
    list_formats(&default_device);

    println!("Other Available Audio Devices:");
    for device in host.output_devices().expect("Failed to access cpal output devices.") {
        if device.name().expect("Failed to access cpal::Device name") != default_name {
            println!("  {}", device.name().expect("Failed to access cpal::Device name"));
            list_formats(&device);
        }
    }
}

fn match_output(host: &cpal::Host, device_name: Option<String>) -> cpal::Device {
    match device_name {
        Some(dn) => {
            let mut cpal_device = None;
            for device in host.output_devices().expect("Failed to access cpal output devices.") {
                if device.name().expect("Failed to access cpal::Device name") == dn {
                    cpal_device = Some(device);
                    break;
                }
            }
            match cpal_device {
                Some(cd) => cd,
                None => {
                    println!("No output sink matching '{}' found.", dn);
                    exit(0)
                }
            }
        },
        None => host.default_output_device().expect("no output device available")
    }
}

impl Open for CpalSink {
    fn open(device_name: Option<String>) -> CpalSink {
        debug!("Using CPAL sink");
        let host = cpal::default_host();

        if device_name == Some("?".to_string()) {
            list_outputs(&host);
            exit(0)
        }

        // buffer for samples from librespot (~100ms)
        let (tx, rx) = sync_channel::<i16>(10 * 2 * 441);

        let event_loop = Arc::new(host.event_loop());

        let ev2 = event_loop.clone();

        thread::spawn(move || {
            ev2.run(move |id, result| {
                let data = match result {
                    Ok(d) => d,
                    Err(err) => {
                        warn!("cpal: an error occurred on stream {:?}: {}", id, err);
                        return;
                    }
                };

                match data {
                    cpal::StreamData::Output { buffer: cpal::UnknownTypeOutputBuffer::I16(mut buffer) } => {
                        for (sample, recv) in buffer.iter_mut().zip(rx.try_iter()) {
                            *sample = recv;
                        }
                    },
                    cpal::StreamData::Output { buffer: cpal::UnknownTypeOutputBuffer::U16(mut buffer) } => {
                        for (sample, recv) in buffer.iter_mut().zip(rx.try_iter()) {
                            *sample = recv.to_sample::<u16>();
                        }
                    },
                    cpal::StreamData::Output { buffer: cpal::UnknownTypeOutputBuffer::F32(mut buffer) } => {
                        for (sample, recv) in buffer.iter_mut().zip(rx.try_iter()) {
                            *sample = recv.to_sample::<f32>();
                        }
                    },
                    _ => (),
                }
            });
        });


        CpalSink {
            host,
            device_name,
            event_loop,
            send: tx,
            stream_id: None,
            resample: None,
        }
    }
}

impl Sink for CpalSink {
    fn start(&mut self) -> io::Result<()> {
        let device = match_output(&self.host, self.device_name.clone());
        let format = device.default_output_format().unwrap();
        let stream_id = self.event_loop.build_output_stream(&device, &format).unwrap();
        self.event_loop.play_stream(stream_id.clone()).unwrap();
        self.stream_id = Some(stream_id);
        self.resample = match format.sample_rate.0 {
            44100 => None,
            sample_rate => {
                debug!("Resampling from 44100 to {:?}", sample_rate);
                Some(ResampleParams{
                    last_frame: [0, 0],
                    target_sample_rate: sample_rate as f64
                })
            },
        };
        Ok(())
    }

    fn stop(&mut self) -> io::Result<()> {
        match self.stream_id.clone() {
            Some(stream_id) => {
                self.event_loop.destroy_stream(stream_id);
                self.stream_id = None;
            },
            None => (),
        }
        Ok(())
    }

    fn write(&mut self, data: &[i16]) -> io::Result<()> {
        match self.resample {
            None => {
                for s in data {
                    let res = self.send.send(*s);
                    if res.is_err() {
                        error!("cpal: cannot write to channel");
                    }
                }
            },
            Some(ref mut params) => {
                // Copy the decoded audio into a Signal.
                let signal = signal::from_interleaved_samples_iter::<_, [i16; 2]>(data.iter().map(|v| *v));
                // Instantiate a Linear interpolator using frame values from the last chunk.
                let interpolator = interpolate::Linear::new(params.last_frame.clone(),
                                                            params.last_frame.clone());
                // Interpolate into a new Signal object.
                let new_signal = signal.from_hz_to_hz(interpolator, 44100 as f64, params.target_sample_rate);

                // Send to the the reciever.
                for frame in new_signal.until_exhausted() {
                    let res = self.send.send(frame[0]);
                    if res.is_err() {
                        error!("cpal: cannot write to channel");
                    }
                    let res = self.send.send(frame[1]);
                    if res.is_err() {
                        error!("cpal: cannot write to channel");
                    }
                    // Store final frame for seamless interpolation into the next chunk.
                    params.last_frame = frame;
                }
            },
        };
        Ok(())
    }
}
