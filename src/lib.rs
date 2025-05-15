#![allow(unused_imports)]
mod flac;

/// * The flac encoder. The `FlacEncoder` is a wrapper for the `FlacEncoderUnmovable` what prevents the structure moves.
pub use crate::flac::{FlacEncoderUnmovable, FlacEncoder};

/// * The flac decoder. The `FlacDecoder` is a wrapper for the `FlacDecoderUnmovable` what prevents the structure moves.
pub use crate::flac::{FlacDecoderUnmovable, FlacDecoder};

/// * The codec options for FLAC
pub mod options {
    pub use crate::flac::{FlacAudioForm, SamplesInfo};
    pub use crate::flac::{FlacCompression, FlacEncoderParams};
}

/// * The objects for you to implement your closure, some is closures' params, some is the return value that your closure should return.
pub mod closure_objects {
    pub use crate::flac::SamplesInfo;
    pub use crate::flac::{FlacReadStatus, FlacInternalDecoderError};
}

/// The errors of this library
pub mod errors {
    pub use crate::flac::FlacError;
    pub use crate::flac::{FlacEncoderError, FlacDecoderError};
    pub use crate::flac::{FlacEncoderErrorCode, FlacDecoderErrorCode};
    pub use crate::flac::{FlacEncoderInitError, FlacDecoderInitError};
    pub use crate::flac::{FlacEncoderInitErrorCode, FlacDecoderInitErrorCode};
}

#[test]
fn test() {
    use std::{io::{self, Read, Write, Seek, SeekFrom, BufReader, BufWriter}, cmp::Ordering, fs::File};

    // Open the FLAC file for decoding using the `BufReader`
    type ReaderType = BufReader<File>;
    let mut reader: ReaderType = BufReader::new(File::open("test.flac").unwrap());

    // Retrieve the file length
    let length = {
        reader.seek(SeekFrom::End(0)).unwrap();
        let ret = reader.stream_position().unwrap();
        reader.seek(SeekFrom::Start(0)).unwrap();
        ret
    };

    // Open the FLAC file for encoding using the `BufWriter`
    type WriterType = BufWriter<File>;
    let mut writer: WriterType = BufWriter::new(File::create("output.flac").unwrap());

    // Prepare to get the samples
    let mut pcm_frames = Vec::<Vec<i16>>::new();

    // There is an encoder to save samples to another FLAC file
    // But currently we don't know the source FLAC file spec (channels, sample rate, etc.)
    // So we just guess it.
    // Let's create the encoder now
    let mut encoder = FlacEncoder::new(
        &mut writer,
        // on_write
        Box::new(|writer: &mut WriterType, data: &[u8]| -> Result<(), io::Error> {
            writer.write_all(data)
        }),
        // on_seek
        Box::new(|writer: &mut WriterType, position: u64| -> Result<(), io::Error> {
            writer.seek(SeekFrom::Start(position))?;
            Ok(())
        }),
        // on_tell
        Box::new(|writer: &mut WriterType| -> Result<u64, io::Error> {
            writer.stream_position()
        }),
        &FlacEncoderParams {
            verify_decoded: false,
            compression: FlacCompression::Level8,
            channels: 2,
            sample_rate: 44100,
            bits_per_sample: 16,
            total_samples_estimate: 0
        }
    ).unwrap();
    encoder.initialize().unwrap();

    // Create a decoder to decode the test file.
    let mut decoder = FlacDecoder::new(
        &mut reader,
        // on_read
        Box::new(|reader: &mut ReaderType, data: &mut [u8]| -> (usize, FlacReadStatus) {
            let to_read = data.len();
            match reader.read(data) {
                Ok(size) => {
                    match size.cmp(&to_read) {
                        Ordering::Equal => (size, FlacReadStatus::GoOn),
                        Ordering::Less => (size, FlacReadStatus::Eof),
                        Ordering::Greater => panic!("`reader.read()` returns a size greater than the desired size."),
                    }
                },
                Err(e) => {
                    eprintln!("on_read(): {:?}", e);
                    (0, FlacReadStatus::Abort)
                }
            }
        }),
        // on_seek
        Box::new(|reader: &mut ReaderType, position: u64| -> Result<(), io::Error> {
            reader.seek(SeekFrom::Start(position))?;
            Ok(())
        }),
        // on_tell
        Box::new(|reader: &mut ReaderType| -> Result<u64, io::Error> {
            reader.stream_position()
        }),
        // on_length
        Box::new(|_reader: &mut ReaderType| -> Result<u64, io::Error>{
            Ok(length)
        }),
        // on_eof
        Box::new(|reader: &mut ReaderType| -> bool {
            reader.stream_position().unwrap() >= length
        }),
        // on_write
        Box::new(|samples: &[Vec<i32>], sample_info: &SamplesInfo| -> Result<(), io::Error>{
            if sample_info.bits_per_sample != 16 {
                panic!("The test function only tests 16-bit per sample FLAC files.")
            }
            let pcm_converted: Vec<Vec<i16>> = samples.iter().map(|frame: &Vec<i32>|{
                frame.into_iter().map(|x32|{*x32 as i16}).collect()
            }).collect();
            pcm_frames.extend(pcm_converted);

            // The encoder wants the `i32` for samples to be encoded so we convert the PCM samples back to `i32` format for the encoder.
            let i32pcm: Vec::<Vec<i32>> = pcm_frames.iter().map(|frame: &Vec<i16>|{
                frame.into_iter().map(|x16|{*x16 as i32}).collect()
            }).collect();
            encoder.write_frames(&i32pcm).unwrap();
            pcm_frames.clear();

            Ok(())
        }),
        // on_error
        Box::new(|error: FlacInternalDecoderError| {
            panic!("{error}");
        }),
        true, // md5_checking
        false, // scale_to_i32_range
        FlacAudioForm::FrameArray
    ).unwrap();

    decoder.decode_all().unwrap();
    decoder.finalize();
    encoder.finalize();
}

