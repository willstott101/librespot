use super::{Open, Sink};
extern crate sample;
extern crate cpal;
extern crate ringbuf;
use std::{io, thread, time};
use std::process::exit;
use std::sync::{Arc};

use self::ringbuf::{RingBuffer, Producer};
use self::sample::{interpolate, signal, Sample, Signal};

struct ResampleParams {
    last_frame: [i16; 2],
    target_sample_rate: f64,
}

pub struct CpalSink {
    device_name: Option<String>,
    event_loop: Arc<cpal::EventLoop>,
    stream_id: Option<cpal::StreamId>,
    send: Producer<i16>,
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

fn list_outputs() {
    let default_device = cpal::default_output_device().unwrap();
    println!("Default Audio Device:\n  {}", default_device.name());
    list_formats(&default_device);

    println!("Other Available Audio Devices:");
    for device in cpal::output_devices() {
        if device.name() != default_device.name() {
            println!("  {}", device.name());
            list_formats(&device);
        }
    }
}

fn match_output(device_name: Option<String>) -> cpal::Device {
    match device_name {
        Some(dn) => {
            let mut cpal_device = None;
            for device in cpal::output_devices() {
                if device.name() == dn {
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
        None => cpal::default_output_device().expect("no output device available")
    }
}

impl Open for CpalSink {
    fn open(device_name: Option<String>) -> CpalSink {
        debug!("Using CPAL sink");

        if device_name == Some("?".to_string()) {
            list_outputs();
            exit(0)
        }

        // buffer for samples from librespot (~100ms)
        let rb = RingBuffer::<i16>::new(10 * 2 * 441);
        let (tx, mut rx) = rb.split();

        let event_loop = Arc::new(cpal::EventLoop::new());

        let ev2 = event_loop.clone();

        thread::spawn(move || {
            ev2.run(move |_stream_id, stream_data| {
                match stream_data {
                    cpal::StreamData::Output { buffer: cpal::UnknownTypeOutputBuffer::I16(mut buffer) } => {
                        println!("ev2 data: I16");
                        match rx.pop_slice(&mut buffer) {
                            Ok(_) => (),
                            Err(_) => (),
                        }
                    },
                    cpal::StreamData::Output { buffer: cpal::UnknownTypeOutputBuffer::U16(mut buffer) } => {
                        for sample in buffer.iter_mut() {
                            match rx.pop() {
                                Ok(v) => *sample = v.to_sample::<u16>(),
                                Err(_) => {
                                    break;
                                },
                            }
                        }
                    },
                    cpal::StreamData::Output { buffer: cpal::UnknownTypeOutputBuffer::F32(mut buffer) } => {
                        for sample in buffer.iter_mut() {
                            match rx.pop() {
                                Ok(v) => *sample = v.to_sample::<f32>(),
                                Err(_) => {
                                    break;
                                },
                            }
                        }
                    },
                    _ => (),
                }
            });
        });

        CpalSink {
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
        let device = match_output(self.device_name.clone());
        let format = device.default_output_format().unwrap();
        let stream_id = self.event_loop.build_output_stream(&device, &format).unwrap();
        self.event_loop.play_stream(stream_id.clone());
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
        // info!("write");
        match self.resample {
            None => {
                let mut start = 0 as usize;
                loop
                {
                    match self.send.push_slice(&data[start..]) {
                        Ok(cnt) => {
                            if (start + cnt) < data.len() {
                                start += cnt;
                            }
                            else {
                                break;
                            }
                        },
                        _ => thread::sleep(time::Duration::from_millis(10)),
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
                    loop
                    {
                        match self.send.push(frame[0]) {
                            Ok(_) => break,
                            Err(_) => thread::sleep(time::Duration::from_millis(10)),
                        }
                    }
                    loop {
                        match self.send.push(frame[1]) {
                            Ok(_) => break,
                            Err(_) => thread::sleep(time::Duration::from_millis(10)),
                        }
                    }
                    // Store final frame for seamless interpolation into the next chunk.
                    params.last_frame = frame;
                }
            },
        };
        Ok(())
    }
}
