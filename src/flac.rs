#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::enum_variant_names)]
#![allow(clippy::map_entry)]

const SHOW_CALLBACKS: bool = false;

use std::{any::Any, io::{Read, Write, Seek}};

use std::io::SeekFrom;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FlacCompression {
    Level0 = 0,
    Level1 = 1,
    Level2 = 2,
    Level3 = 3,
    Level4 = 4,
    Level5 = 5,
    Level6 = 6,
    Level7 = 7,
    Level8 = 8
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlacEncoderParams {
    pub verify_decoded: bool,
    pub compression: FlacCompression,
    pub channels: u16,
    pub sample_rate: u32,
    pub bits_per_sample: u32,
    pub total_samples_estimate: u64,
}

impl FlacEncoderParams {
    pub fn new() -> Self {
        Self {
            verify_decoded: false,
            compression: FlacCompression::Level5,
            channels: 2,
            sample_rate: 44100,
            bits_per_sample: 16,
            total_samples_estimate: 0,
        }
    }
}

impl Default for FlacEncoderParams {
    fn default() -> Self {
        Self::new()
    }
}

use std::{borrow::Cow, io::{self, ErrorKind}, fmt::{self, Debug, Display, Formatter}, slice, ffi::{CStr, c_void}, ptr, collections::BTreeMap};

#[cfg(feature = "id3")]
use id3::{self, TagLike};

use libflac_sys::*;

pub trait FlacError: Any {
    fn as_any(&self) -> &dyn Any;
    fn get_code(&self) -> u32;
    fn get_message(&self) -> &'static str;
    fn get_function(&self) -> &'static str;
    fn get_message_from_code(&self) -> &'static str;

    fn format(&self, f: &mut Formatter) -> fmt::Result {
        let code = self.get_code();
        let message = self.get_message();
        let function = self.get_function();
        write!(f, "Code: {code}, function: {function}, message: {message}")?;
        Ok(())
    }
}

macro_rules! impl_FlacError {
    ($error:ty) => {
        impl FlacError for $error {
            fn as_any(&self) -> &dyn Any {self}
            fn get_code(&self) -> u32 {self.code}
            fn get_message(&self) -> &'static str {self.message}
            fn get_function(&self) -> &'static str {self.function}
            fn get_message_from_code(&self) -> &'static str {
                Self::get_message_from_code(self.get_code())
            }
        }

        impl std::error::Error for $error {}

        impl Display for $error {
            fn fmt(&self, f: &mut Formatter) -> fmt::Result {
                <$error as FlacError>::format(self, f)
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FlacEncoderError {
    pub code: u32,
    pub message: &'static str,
    pub function: &'static str,
}

impl FlacEncoderError {
    pub fn new(code: u32, function: &'static str) -> Self {
        Self {
            code,
            message: Self::get_message_from_code(code),
            function,
        }
    }

    pub fn get_message_from_code(code: u32) -> &'static str {
        unsafe {
            CStr::from_ptr(*FLAC__StreamEncoderStateString.as_ptr().add(code as usize)).to_str().unwrap()
        }
    }
}

impl_FlacError!(FlacEncoderError);

#[derive(Debug, Clone, Copy)]
pub enum FlacEncoderErrorCode {
    StreamEncoderOk = FLAC__STREAM_ENCODER_OK as isize,
    StreamEncoderUninitialized = FLAC__STREAM_ENCODER_UNINITIALIZED as isize,
    StreamEncoderOggError = FLAC__STREAM_ENCODER_OGG_ERROR as isize,
    StreamEncoderVerifyDecoderError = FLAC__STREAM_ENCODER_VERIFY_DECODER_ERROR as isize,
    StreamEncoderVerifyMismatchInAudioData = FLAC__STREAM_ENCODER_VERIFY_MISMATCH_IN_AUDIO_DATA as isize,
    StreamEncoderClientError = FLAC__STREAM_ENCODER_CLIENT_ERROR as isize,
    StreamEncoderIOError = FLAC__STREAM_ENCODER_IO_ERROR as isize,
    StreamEncoderFramingError = FLAC__STREAM_ENCODER_FRAMING_ERROR as isize,
    StreamEncoderMemoryAllocationError = FLAC__STREAM_ENCODER_MEMORY_ALLOCATION_ERROR as isize,
}

impl Display for FlacEncoderErrorCode {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Self::StreamEncoderOk => write!(f, "The encoder is in the normal OK state and samples can be processed."),
            Self::StreamEncoderUninitialized => write!(f, "The encoder is in the uninitialized state; one of the FLAC__stream_encoder_init_*() functions must be called before samples can be processed."),
            Self::StreamEncoderOggError => write!(f, "An error occurred in the underlying Ogg layer."),
            Self::StreamEncoderVerifyDecoderError => write!(f, "An error occurred in the underlying verify stream decoder; check FLAC__stream_encoder_get_verify_decoder_state()."),
            Self::StreamEncoderVerifyMismatchInAudioData => write!(f, "The verify decoder detected a mismatch between the original audio signal and the decoded audio signal."),
            Self::StreamEncoderClientError => write!(f, "One of the callbacks returned a fatal error."),
            Self::StreamEncoderIOError => write!(f, "An I/O error occurred while opening/reading/writing a file. Check errno."),
            Self::StreamEncoderFramingError => write!(f, "An error occurred while writing the stream; usually, the write_callback returned an error."),
            Self::StreamEncoderMemoryAllocationError => write!(f, "Memory allocation failed."),
        }
    }
}

impl From<u32> for FlacEncoderErrorCode {
    fn from(code: u32) -> Self {
        use FlacEncoderErrorCode::*;
        match code {
            FLAC__STREAM_ENCODER_OK => StreamEncoderOk,
            FLAC__STREAM_ENCODER_UNINITIALIZED => StreamEncoderUninitialized,
            FLAC__STREAM_ENCODER_OGG_ERROR => StreamEncoderOggError,
            FLAC__STREAM_ENCODER_VERIFY_DECODER_ERROR => StreamEncoderVerifyDecoderError,
            FLAC__STREAM_ENCODER_VERIFY_MISMATCH_IN_AUDIO_DATA => StreamEncoderVerifyMismatchInAudioData,
            FLAC__STREAM_ENCODER_CLIENT_ERROR => StreamEncoderClientError,
            FLAC__STREAM_ENCODER_IO_ERROR => StreamEncoderIOError,
            FLAC__STREAM_ENCODER_FRAMING_ERROR  => StreamEncoderFramingError,
            FLAC__STREAM_ENCODER_MEMORY_ALLOCATION_ERROR => StreamEncoderMemoryAllocationError,
            o => panic!("Not an encoder error code: {o}."),
        }
    }
}

impl std::error::Error for FlacEncoderErrorCode {}

#[derive(Debug, Clone, Copy)]
pub struct FlacEncoderInitError {
    pub code: u32,
    pub message: &'static str,
    pub function: &'static str,
}

impl FlacEncoderInitError {
    pub fn new(code: u32, function: &'static str) -> Self {
        Self {
            code,
            message: Self::get_message_from_code(code),
            function,
        }
    }

    pub fn get_message_from_code(code: u32) -> &'static str {
        unsafe {
            CStr::from_ptr(*FLAC__StreamEncoderInitStatusString.as_ptr().add(code as usize)).to_str().unwrap()
        }
    }
}

impl_FlacError!(FlacEncoderInitError);

#[derive(Debug, Clone, Copy)]
pub enum FlacEncoderInitErrorCode {
    StreamEncoderInitStatusOk = FLAC__STREAM_ENCODER_INIT_STATUS_OK as isize,
    StreamEncoderInitStatusEncoderError = FLAC__STREAM_ENCODER_INIT_STATUS_ENCODER_ERROR as isize,
    StreamEncoderInitStatusUnsupportedContainer = FLAC__STREAM_ENCODER_INIT_STATUS_UNSUPPORTED_CONTAINER as isize,
    StreamEncoderInitStatusInvalidCallbacks = FLAC__STREAM_ENCODER_INIT_STATUS_INVALID_CALLBACKS as isize,
    StreamEncoderInitStatusInvalidNumberOfChannels = FLAC__STREAM_ENCODER_INIT_STATUS_INVALID_NUMBER_OF_CHANNELS as isize,
    StreamEncoderInitStatusInvalidBitsPerSample = FLAC__STREAM_ENCODER_INIT_STATUS_INVALID_BITS_PER_SAMPLE as isize,
    StreamEncoderInitStatusInvalidSampleRate = FLAC__STREAM_ENCODER_INIT_STATUS_INVALID_SAMPLE_RATE as isize,
    StreamEncoderInitStatusInvalidBlockSize = FLAC__STREAM_ENCODER_INIT_STATUS_INVALID_BLOCK_SIZE as isize,
    StreamEncoderInitStatusInvalidMaxLpcOrder = FLAC__STREAM_ENCODER_INIT_STATUS_INVALID_MAX_LPC_ORDER as isize,
    StreamEncoderInitStatusInvalidQlpCoeffPrecision = FLAC__STREAM_ENCODER_INIT_STATUS_INVALID_QLP_COEFF_PRECISION as isize,
    StreamEncoderInitStatusBlockSizeTooSmallForLpcOrder = FLAC__STREAM_ENCODER_INIT_STATUS_BLOCK_SIZE_TOO_SMALL_FOR_LPC_ORDER as isize,
    StreamEncoderInitStatusNotStreamable = FLAC__STREAM_ENCODER_INIT_STATUS_NOT_STREAMABLE as isize,
    StreamEncoderInitStatusInvalidMetadata = FLAC__STREAM_ENCODER_INIT_STATUS_INVALID_METADATA as isize,
    StreamEncoderInitStatusAlreadyInitialized = FLAC__STREAM_ENCODER_INIT_STATUS_ALREADY_INITIALIZED as isize,
}

impl Display for FlacEncoderInitErrorCode {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Self::StreamEncoderInitStatusOk => write!(f, "Initialization was successful."),
            Self::StreamEncoderInitStatusEncoderError => write!(f, "General failure to set up encoder; call FLAC__stream_encoder_get_state() for cause."),
            Self::StreamEncoderInitStatusUnsupportedContainer => write!(f, "The library was not compiled with support for the given container format."),
            Self::StreamEncoderInitStatusInvalidCallbacks => write!(f, "A required callback was not supplied."),
            Self::StreamEncoderInitStatusInvalidNumberOfChannels => write!(f, "The encoder has an invalid setting for number of channels."),
            Self::StreamEncoderInitStatusInvalidBitsPerSample => write!(f, "The encoder has an invalid setting for bits-per-sample. FLAC supports 4-32 bps."),
            Self::StreamEncoderInitStatusInvalidSampleRate => write!(f, "The encoder has an invalid setting for the input sample rate."),
            Self::StreamEncoderInitStatusInvalidBlockSize => write!(f, "The encoder has an invalid setting for the block size."),
            Self::StreamEncoderInitStatusInvalidMaxLpcOrder => write!(f, "The encoder has an invalid setting for the maximum LPC order."),
            Self::StreamEncoderInitStatusInvalidQlpCoeffPrecision => write!(f, "The encoder has an invalid setting for the precision of the quantized linear predictor coefficients."),
            Self::StreamEncoderInitStatusBlockSizeTooSmallForLpcOrder => write!(f, "The specified block size is less than the maximum LPC order."),
            Self::StreamEncoderInitStatusNotStreamable => write!(f, "The encoder is bound to the Subset but other settings violate it."),
            Self::StreamEncoderInitStatusInvalidMetadata => write!(f, "The metadata input to the encoder is invalid, in one of the following ways:\n\n* FLAC__stream_encoder_set_metadata() was called with a null pointer but a block count > 0\n* One of the metadata blocks contains an undefined type\n* It contains an illegal CUESHEET as checked by FLAC__format_cuesheet_is_legal()\n* It contains an illegal SEEKTABLE as checked by FLAC__format_seektable_is_legal()\n* It contains more than one SEEKTABLE block or more than one VORBIS_COMMENT block\n* FLAC__STREAM_ENCODER_INIT_STATUS_ALREADY_INITIALIZED\n* FLAC__stream_encoder_init_*() was called when the encoder was already initialized, usually because FLAC__stream_encoder_finish() was not called."),
            Self::StreamEncoderInitStatusAlreadyInitialized => write!(f, "FLAC__stream_encoder_init_*() was called when the encoder was already initialized, usually because FLAC__stream_encoder_finish() was not called."),
        }
    }
}

impl From<u32> for FlacEncoderInitErrorCode {
    fn from(code: u32) -> Self {
        use FlacEncoderInitErrorCode::*;
        match code {
            FLAC__STREAM_ENCODER_INIT_STATUS_OK => StreamEncoderInitStatusOk,
            FLAC__STREAM_ENCODER_INIT_STATUS_ENCODER_ERROR => StreamEncoderInitStatusEncoderError,
            FLAC__STREAM_ENCODER_INIT_STATUS_UNSUPPORTED_CONTAINER => StreamEncoderInitStatusUnsupportedContainer,
            FLAC__STREAM_ENCODER_INIT_STATUS_INVALID_CALLBACKS => StreamEncoderInitStatusInvalidCallbacks,
            FLAC__STREAM_ENCODER_INIT_STATUS_INVALID_NUMBER_OF_CHANNELS => StreamEncoderInitStatusInvalidNumberOfChannels,
            FLAC__STREAM_ENCODER_INIT_STATUS_INVALID_BITS_PER_SAMPLE => StreamEncoderInitStatusInvalidBitsPerSample,
            FLAC__STREAM_ENCODER_INIT_STATUS_INVALID_SAMPLE_RATE => StreamEncoderInitStatusInvalidSampleRate,
            FLAC__STREAM_ENCODER_INIT_STATUS_INVALID_BLOCK_SIZE => StreamEncoderInitStatusInvalidBlockSize,
            FLAC__STREAM_ENCODER_INIT_STATUS_INVALID_MAX_LPC_ORDER => StreamEncoderInitStatusInvalidMaxLpcOrder,
            FLAC__STREAM_ENCODER_INIT_STATUS_INVALID_QLP_COEFF_PRECISION => StreamEncoderInitStatusInvalidQlpCoeffPrecision,
            FLAC__STREAM_ENCODER_INIT_STATUS_BLOCK_SIZE_TOO_SMALL_FOR_LPC_ORDER => StreamEncoderInitStatusBlockSizeTooSmallForLpcOrder,
            FLAC__STREAM_ENCODER_INIT_STATUS_NOT_STREAMABLE => StreamEncoderInitStatusNotStreamable,
            FLAC__STREAM_ENCODER_INIT_STATUS_INVALID_METADATA => StreamEncoderInitStatusInvalidMetadata,
            FLAC__STREAM_ENCODER_INIT_STATUS_ALREADY_INITIALIZED => StreamEncoderInitStatusAlreadyInitialized,
            o => panic!("Not an encoder init error code: {o}."),
        }
    }
}

impl std::error::Error for FlacEncoderInitErrorCode {}

impl From<FlacEncoderError> for FlacEncoderInitError {
    fn from(err: FlacEncoderError) -> Self {
        Self {
            code: err.code,
            message: err.message,
            function: err.function,
        }
    }
}

impl From<FlacEncoderInitError> for FlacEncoderError {
    fn from(err: FlacEncoderInitError) -> Self {
        Self {
            code: err.code,
            message: err.message,
            function: err.function,
        }
    }
}

pub const COMMENT_KEYS: [&str; 33] = [
    "ACTOR",
    "ALBUM",
    "ARTIST",
    "ALBUMARTIST",
    "COMMENT",
    "COMPOSER",
    "CONTACT",
    "COPYRIGHT",
    "COVERART",
    "COVERARTMIME",
    "DATE",
    "DESCRIPTION",
    "DIRECTOR",
    "ENCODED_BY",
    "ENCODED_USING",
    "ENCODER",
    "ENCODER_OPTIONS",
    "GENRE",
    "ISRC",
    "LICENSE",
    "LOCATION",
    "ORGANIZATION",
    "PERFORMER",
    "PRODUCER",
    "REPLAYGAIN_ALBUM_GAIN",
    "REPLAYGAIN_ALBUM_PEAK",
    "REPLAYGAIN_TRACK_GAIN",
    "REPLAYGAIN_TRACK_PEAK",
    "TITLE",
    "TRACKNUMBER",
    "TRACKTOTAL",
    "VERSION",
    "vendor"
];

#[derive(Clone)]
pub struct PictureData {
    pub picture: Vec<u8>,
    pub mime_type: String,
    pub description: String,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub colors: u32,
}

impl Debug for PictureData {
    fn fmt(&self, fmt: &mut Formatter) -> fmt::Result {
        fmt.debug_struct("PictureData")
            .field("picture", &format_args!("[u8; {}]", self.picture.len()))
            .field("mime_type", &self.mime_type)
            .field("description", &self.description)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("depth", &self.depth)
            .field("colors", &self.colors)
            .finish()
    }
}

impl PictureData {
    pub fn new() -> Self {
        Self {
            picture: Vec::<u8>::new(),
            mime_type: "".to_owned(),
            description: "".to_owned(),
            width: 0,
            height: 0,
            depth: 0,
            colors: 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.picture.is_empty()
    }
}

impl Default for PictureData {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
#[repr(C)]
struct FlacMetadata {
    // https://xiph.org/flac/api/group__flac__metadata__object.html
    metadata: *mut FLAC__StreamMetadata,
}

#[derive(Debug)]
#[repr(C)]
struct FlacCueTrackWrap {
    track: *mut FLAC__StreamMetadata_CueSheet_Track,
}

impl FlacCueTrackWrap {
    pub fn new() -> Result<Self, FlacEncoderError> {
        let ret = Self {
            track: unsafe {FLAC__metadata_object_cuesheet_track_new()},
        };
        if ret.track.is_null() {
            Err(FlacEncoderError::new(FLAC__STREAM_ENCODER_MEMORY_ALLOCATION_ERROR, "FLAC__metadata_object_cuesheet_track_new"))
        } else {
            Ok(ret)
        }
    }

    pub fn get_ref_mut(&mut self) -> &mut FLAC__StreamMetadata_CueSheet_Track {
        unsafe {&mut *self.track}
    }

    pub fn get_ptr(&self) -> *const FLAC__StreamMetadata_CueSheet_Track{
        self.track as *const FLAC__StreamMetadata_CueSheet_Track
    }

    pub fn get_mut_ptr(&self) -> *mut FLAC__StreamMetadata_CueSheet_Track{
        self.track
    }
}

impl Default for FlacCueTrackWrap {
    fn default() -> Self {
        Self {
            track: ptr::null_mut(),
        }
    }
}

impl Drop for FlacCueTrackWrap {
    fn drop(&mut self) {
        if !self.track.is_null() {
            unsafe {FLAC__metadata_object_cuesheet_track_delete(self.track)};
            self.track = ptr::null_mut();
        }
    }
}

fn make_sz(s: &str) -> String {
    let mut s = s.to_owned();
    if !s.ends_with('\0') {s.push('\0');}
    s
}

#[derive(Debug, Clone, Copy)]
pub enum FlacTrackType {
    Audio,
    NonAudio,
}

impl Display for FlacTrackType {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Self::Audio => write!(f, "audio"),
            Self::NonAudio => write!(f, "non-audio"),
        }
    }
}

/// offset: Offset in samples, relative to the track offset, of the index point.
/// number: The index point number
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct FlacCueSheetIndex {
    pub offset: u64,
    pub number: u8,
}

#[derive(Clone)]
#[repr(C)]
pub struct FlacCueTrack {
    pub offset: u64, // in samples
    pub track_no: u8,
    pub isrc: [i8; 13],
    pub type_: FlacTrackType,
    pub pre_emphasis: bool,
    pub indices: Vec<FlacCueSheetIndex>,
}

impl FlacCueTrack {
    pub fn get_isrc(&self) -> String {
        String::from_utf8_lossy(&self.isrc.iter().map(|c|{*c as u8}).collect::<Vec<u8>>()).to_string()
    }
}

impl Debug for FlacCueTrack {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        f.debug_struct("FlacCueTrack")
            .field("offset", &self.offset)
            .field("track_no", &self.track_no)
            .field("isrc", &self.get_isrc())
            .field("type_", &self.type_)
            .field("pre_emphasis", &self.pre_emphasis)
            .field("indices", &self.indices)
            .finish()
    }
}

impl Display for FlacCueTrack {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        f.debug_struct("FlacCueTrack")
            .field("offset", &self.offset)
            .field("track_no", &self.track_no)
            .field("isrc", &self.get_isrc())
            .field("type_", &self.type_)
            .field("pre_emphasis", &self.pre_emphasis)
            .field("indices", &self.indices)
            .finish()
    }
}

#[derive(Clone)]
pub struct FlacCueSheet {
    pub media_catalog_number: [i8; 129],
    pub lead_in: u64,
    pub is_cd: bool,
    pub tracks: BTreeMap<u8, FlacCueTrack>,
}

impl FlacCueSheet {
    pub fn get_media_catalog_number(&self) -> String {
        String::from_utf8_lossy(&self.media_catalog_number.iter().map(|c|{*c as u8}).collect::<Vec<u8>>()).to_string()
    }
}

impl Debug for FlacCueSheet {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        f.debug_struct("FlacCueSheet")
            .field("media_catalog_number", &self.get_media_catalog_number())
            .field("lead_in", &self.lead_in)
            .field("is_cd", &self.is_cd)
            .field("tracks", &self.tracks)
            .finish()
    }
}

impl Display for FlacCueSheet {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        f.debug_struct("FlacCueSheet")
            .field("media_catalog_number", &self.get_media_catalog_number())
            .field("lead_in", &self.lead_in)
            .field("is_cd", &self.is_cd)
            .field("tracks", &self.tracks)
            .finish()
    }
}

impl FlacMetadata {
    pub fn new_vorbis_comment() -> Result<Self, FlacEncoderError> {
        let ret = Self {
            metadata: unsafe {FLAC__metadata_object_new(FLAC__METADATA_TYPE_VORBIS_COMMENT)},
        };
        if ret.metadata.is_null() {
            Err(FlacEncoderError::new(FLAC__STREAM_ENCODER_MEMORY_ALLOCATION_ERROR, "FLAC__metadata_object_new(FLAC__METADATA_TYPE_VORBIS_COMMENT)"))
        } else {
            Ok(ret)
        }
    }

    pub fn new_cue_sheet() -> Result<Self, FlacEncoderError> {
        let ret = Self {
            metadata: unsafe {FLAC__metadata_object_new(FLAC__METADATA_TYPE_CUESHEET)},
        };
        if ret.metadata.is_null() {
            Err(FlacEncoderError::new(FLAC__STREAM_ENCODER_MEMORY_ALLOCATION_ERROR, "FLAC__metadata_object_new(FLAC__METADATA_TYPE_CUESHEET)"))
        } else {
            Ok(ret)
        }
    }

    pub fn new_picture() -> Result<Self, FlacEncoderError> {
        let ret = Self {
            metadata: unsafe {FLAC__metadata_object_new(FLAC__METADATA_TYPE_PICTURE)},
        };
        if ret.metadata.is_null() {
            Err(FlacEncoderError::new(FLAC__STREAM_ENCODER_MEMORY_ALLOCATION_ERROR, "FLAC__metadata_object_new(FLAC__METADATA_TYPE_PICTURE)"))
        } else {
            Ok(ret)
        }
    }

    pub fn insert_comments(&self, key: &'static str, value: &str) -> Result<(), FlacEncoderError> {
        unsafe {
            // ATTENTION:
            // Any strings to be added to the entry must be NUL terminated.
            // Or you can see the `FLAC__STREAM_ENCODER_MEMORY_ALLOCATION_ERROR` due to the failure to find the NUL terminator.
            let szkey = make_sz(key);
            let szvalue = make_sz(value);
            let mut entry = FLAC__StreamMetadata_VorbisComment_Entry{length: 0, entry: ptr::null_mut()};
            if FLAC__metadata_object_vorbiscomment_entry_from_name_value_pair (
                &mut entry as *mut FLAC__StreamMetadata_VorbisComment_Entry,
                szkey.as_ptr() as *mut i8,
                szvalue.as_ptr() as *mut i8
            ) == 0 {
                eprintln!("On set comment {key}: {value}: {:?}", FlacEncoderError::new(FLAC__STREAM_ENCODER_MEMORY_ALLOCATION_ERROR, "FLAC__metadata_object_vorbiscomment_entry_from_name_value_pair"));
            }
            if FLAC__metadata_object_vorbiscomment_append_comment(self.metadata, entry, 0) == 0 {
                eprintln!("On set comment {key}: {value}: {:?}", FlacEncoderError::new(FLAC__STREAM_ENCODER_MEMORY_ALLOCATION_ERROR, "FLAC__metadata_object_vorbiscomment_append_comment"));
            }
        }
        Ok(())
    }

    pub fn insert_cue_track(&mut self, track_no: u8, cue_track: &FlacCueTrack) -> Result<(), FlacEncoderError> {
        unsafe {
            let mut track = FlacCueTrackWrap::new()?;
            let track_data = track.get_ref_mut();
            track_data.offset = cue_track.offset;
            track_data.number = track_no;
            track_data.isrc = cue_track.isrc;
            track_data.set_type(match cue_track.type_ {
                FlacTrackType::Audio => 0,
                FlacTrackType::NonAudio => 1,
            });
            track_data.set_pre_emphasis(match cue_track.pre_emphasis {
                true => 1,
                false => 0,
            });
            track_data.num_indices = cue_track.indices.len() as u8;
            let mut indices: Vec<FLAC__StreamMetadata_CueSheet_Index> = cue_track.indices.iter().map(|index| -> FLAC__StreamMetadata_CueSheet_Index {
                FLAC__StreamMetadata_CueSheet_Index {
                    offset: index.offset,
                    number: index.number,
                }
            }).collect();
            track_data.indices = indices.as_mut_ptr();
            if FLAC__metadata_object_cuesheet_set_track(self.metadata, track_no as u32, track.get_mut_ptr(), 1) == 0 {
                eprintln!("Failed to create new cuesheet track for {track_no} {cue_track}:  {:?}", FlacEncoderError::new(FLAC__STREAM_ENCODER_MEMORY_ALLOCATION_ERROR, "FLAC__metadata_object_cuesheet_set_track"));
            }
        }
        Ok(())
    }

    pub fn set_picture(&mut self, picture_binary: &mut [u8], description: &mut str, mime_type: &mut str) -> Result<(), FlacEncoderError> {
        let mut desc_sz = make_sz(description);
        let mut mime_sz = make_sz(mime_type);
        unsafe {
            if FLAC__metadata_object_picture_set_data(self.metadata, picture_binary.as_mut_ptr(), picture_binary.len() as u32, 0) == 0 {
                Err(FlacEncoderError::new(FLAC__STREAM_ENCODER_MEMORY_ALLOCATION_ERROR, "FLAC__metadata_object_picture_set_data"))
            } else if FLAC__metadata_object_picture_set_mime_type(self.metadata, desc_sz.as_mut_ptr() as *mut i8, 0) == 0 {
                Err(FlacEncoderError::new(FLAC__STREAM_ENCODER_MEMORY_ALLOCATION_ERROR, "FLAC__metadata_object_picture_set_mime_type"))
            } else if FLAC__metadata_object_picture_set_description(self.metadata, mime_sz.as_mut_ptr(), 0) == 0 {
                Err(FlacEncoderError::new(FLAC__STREAM_ENCODER_MEMORY_ALLOCATION_ERROR, "FLAC__metadata_object_picture_set_description"))
            } else {
                Ok(())
            }
        }
    }
}

impl Default for FlacMetadata {
    fn default() -> Self {
        Self {
            metadata: ptr::null_mut(),
        }
    }
}

impl Drop for FlacMetadata {
    fn drop(&mut self) {
        if !self.metadata.is_null() {
            unsafe {FLAC__metadata_object_delete(self.metadata)};
            self.metadata = ptr::null_mut();
        }
    }
}

pub struct FlacEncoderUnmovable<'a, WriteSeek>
where
    WriteSeek: Write + Seek + Debug {
    // https://xiph.org/flac/api/group__flac__stream__encoder.html
    encoder: *mut FLAC__StreamEncoder,
    metadata: Vec<FlacMetadata>,
    encoder_initialized: bool,
    params: FlacEncoderParams,
    writer: &'a mut WriteSeek,
    on_write: Box<dyn FnMut(&mut WriteSeek, &[u8]) -> Result<(), io::Error> + 'a>,
    on_seek: Box<dyn FnMut(&mut WriteSeek, u64) -> Result<(), io::Error> + 'a>,
    on_tell: Box<dyn FnMut(&mut WriteSeek) -> Result<u64, io::Error> + 'a>,
    comments: BTreeMap<&'static str, String>,
    cue_sheets: Vec<FlacCueSheet>,
    pictures: Vec<PictureData>,
    finished: bool,
}

impl<'a, WriteSeek> FlacEncoderUnmovable<'a, WriteSeek>
where
    WriteSeek: Write + Seek + Debug {
    pub fn new(
        writer: &'a mut WriteSeek,
        on_write: Box<dyn FnMut(&mut WriteSeek, &[u8]) -> Result<(), io::Error> + 'a>,
        on_seek: Box<dyn FnMut(&mut WriteSeek, u64) -> Result<(), io::Error> + 'a>,
        on_tell: Box<dyn FnMut(&mut WriteSeek) -> Result<u64, io::Error> + 'a>,
        params: &FlacEncoderParams
    ) -> Result<Self, FlacEncoderError> {
        let ret = Self {
            encoder: unsafe {FLAC__stream_encoder_new()},
            metadata: Vec::<FlacMetadata>::new(),
            encoder_initialized: false,
            params: *params,
            writer,
            on_write,
            on_seek,
            on_tell,
            comments: BTreeMap::new(),
            cue_sheets: Vec::new(),
            pictures: Vec::<PictureData>::new(),
            finished: false,
        };
        if ret.encoder.is_null() {
            Err(FlacEncoderError::new(FLAC__STREAM_ENCODER_MEMORY_ALLOCATION_ERROR, "FLAC__stream_encoder_new"))
        } else {
            Ok(ret)
        }
    }

    pub fn get_status_as_result(&self, function: &'static str) -> Result<(), FlacEncoderError> {
        let code = unsafe {FLAC__stream_encoder_get_state(self.encoder)};
        if code == 0 {
            Ok(())
        } else {
            Err(FlacEncoderError::new(code, function))
        }
    }

    pub fn get_status_as_error(&self, function: &'static str) -> Result<(), FlacEncoderError> {
        let code = unsafe {FLAC__stream_encoder_get_state(self.encoder)};
        Err(FlacEncoderError::new(code, function))
    }

    pub fn as_ptr(&self) -> *const Self {
        self as *const Self
    }

    pub fn as_mut_ptr(&mut self) -> *mut Self {
        self as *mut Self
    }

    pub fn insert_comments(&mut self, key: &'static str, value: &str) -> Result<(), FlacEncoderInitError> {
        if self.encoder_initialized {
            Err(FlacEncoderInitError::new(FLAC__STREAM_ENCODER_INIT_STATUS_ALREADY_INITIALIZED, "FlacEncoderUnmovable::insert_comments"))
        } else {
            if let Some(old_value) = self.comments.insert(key, value.to_owned()) {
                eprintln!("\"{key}\" is changed to \"{value}\" from \"{old_value}\"");
            }
            Ok(())
        }
    }

    pub fn insert_cue_sheet(&mut self, cue_sheet: &FlacCueSheet) -> Result<(), FlacEncoderInitError> {
        if self.encoder_initialized {
            Err(FlacEncoderInitError::new(FLAC__STREAM_ENCODER_INIT_STATUS_ALREADY_INITIALIZED, "FlacEncoderUnmovable::insert_cue_track"))
        } else {
            self.cue_sheets.push(cue_sheet.clone());
            Ok(())
        }
    }

    pub fn add_picture(&mut self, picture_binary: &[u8], description: &str, mime_type: &str, width: u32, height: u32, depth: u32, colors: u32) -> Result<(), FlacEncoderInitError> {
        if self.encoder_initialized {
            Err(FlacEncoderInitError::new(FLAC__STREAM_ENCODER_INIT_STATUS_ALREADY_INITIALIZED, "FlacEncoderUnmovable::set_picture"))
        } else {
            self.pictures.push(PictureData{
                picture: picture_binary.to_vec(),
                description: description.to_owned(),
                mime_type: mime_type.to_owned(),
                width,
                height,
                depth,
                colors
            });
            Ok(())
        }
    }

    #[cfg(feature = "id3")]
    pub fn inherit_metadata_from_id3(&mut self, tag: &id3::Tag) -> Result<(), FlacEncoderInitError> {
        if let Some(artist) = tag.artist() {self.insert_comments("ARTIST", artist)?;}
        if let Some(album) = tag.album() {self.insert_comments("ALBUM", album)?;}
        if let Some(title) = tag.title() {self.insert_comments("TITLE", title)?;}
        if let Some(genre) = tag.genre() {self.insert_comments("GENRE", genre)?;}
        for picture in tag.pictures() {
            self.add_picture(&picture.data, &picture.description, &picture.mime_type, 0, 0, 0, 0)?;
        }
        let comm_str = tag.comments().enumerate().map(|(i, comment)| -> String {
            let lang = &comment.lang;
            let desc = &comment.description;
            let text = &comment.text;
            format!("Comment {i}:\n\tlang: {lang}\n\tdesc: {desc}\n\ttext: {text}")
        }).collect::<Vec<String>>().join("\n");
        if !comm_str.is_empty() {self.insert_comments("COMMENT", &comm_str)?;}
        Ok(())
    }

    pub fn initialize(&mut self) -> Result<(), FlacEncoderError> {
        if self.encoder_initialized {
            return Err(FlacEncoderInitError::new(FLAC__STREAM_ENCODER_INIT_STATUS_ALREADY_INITIALIZED, "FlacEncoderUnmovable::init").into())
        }
        unsafe {
            if FLAC__stream_encoder_set_verify(self.encoder, if self.params.verify_decoded {1} else {0}) == 0 {
                return self.get_status_as_error("FLAC__stream_encoder_set_verify");
            }
            if FLAC__stream_encoder_set_compression_level(self.encoder, self.params.compression as u32) == 0 {
                return self.get_status_as_error("FLAC__stream_encoder_set_compression_level");
            }
            if FLAC__stream_encoder_set_channels(self.encoder, self.params.channels as u32) == 0 {
                return self.get_status_as_error("FLAC__stream_encoder_set_channels");
            }
            if FLAC__stream_encoder_set_bits_per_sample(self.encoder, self.params.bits_per_sample) == 0 {
                return self.get_status_as_error("FLAC__stream_encoder_set_bits_per_sample");
            }
            if FLAC__stream_encoder_set_sample_rate(self.encoder, self.params.sample_rate) == 0 {
                return self.get_status_as_error("FLAC__stream_encoder_set_sample_rate");
            }
            if self.params.total_samples_estimate > 0 && FLAC__stream_encoder_set_total_samples_estimate(self.encoder, self.params.total_samples_estimate) == 0 {
                return self.get_status_as_error("FLAC__stream_encoder_set_total_samples_estimate");
            }

            let set_metadata: Result<(), FlacEncoderError> = {
                if !self.comments.is_empty() {
                    let metadata = FlacMetadata::new_vorbis_comment()?;
                    for (key, value) in self.comments.iter() {
                        metadata.insert_comments(key, value)?;
                    }
                    self.metadata.push(metadata);
                }
                for cue_sheet in self.cue_sheets.iter() {
                    let mut metadata = FlacMetadata::new_cue_sheet()?;
                    for (track_no, cue_track) in cue_sheet.tracks.iter() {
                        metadata.insert_cue_track(*track_no, cue_track)?;
                    }
                    self.metadata.push(metadata);
                }
                for picture in self.pictures.iter_mut() {
                    let mut metadata = FlacMetadata::new_picture()?;
                    metadata.set_picture(&mut picture.picture, &mut picture.description, &mut picture.mime_type)?;
                    self.metadata.push(metadata);
                }
                if !self.metadata.is_empty() {
                    if FLAC__stream_encoder_set_metadata(self.encoder, self.metadata.as_mut_ptr() as *mut *mut FLAC__StreamMetadata, self.metadata.len() as u32) == 0 {
                        Err(FlacEncoderError::new(FLAC__STREAM_ENCODER_INIT_STATUS_ALREADY_INITIALIZED, "FLAC__stream_encoder_set_metadata"))
                    } else {
                        Ok(())
                    }
                } else {
                    Ok(())
                }
            };
            if let Err(e) = set_metadata {
                eprintln!("When setting the metadata: {:?}", e);
            }
            let ret = FLAC__stream_encoder_init_stream(self.encoder,
                Some(Self::write_callback),
                Some(Self::seek_callback),
                Some(Self::tell_callback),
                Some(Self::metadata_callback),
                self.as_mut_ptr() as *mut c_void,
            );
            if ret != 0 {
                return Err(FlacEncoderInitError::new(ret, "FLAC__stream_encoder_init_stream").into());
            } else {
                self.encoder_initialized = true;
            }
        }
        self.finished = false;
        self.get_status_as_result("FlacEncoderUnmovable::Init()")
    }

    pub fn get_params(&self) -> FlacEncoderParams {
        self.params
    }

    unsafe extern "C" fn write_callback(_encoder: *const FLAC__StreamEncoder, buffer: *const u8, bytes: usize, _samples: u32, _current_frame: u32, client_data: *mut c_void) -> u32 {
        #[cfg(debug_assertions)]
        if SHOW_CALLBACKS {println!("write_callback([u8; {bytes}])");}
        let this = unsafe {&mut *(client_data as *mut Self)};
        match (this.on_write)(this.writer, unsafe {slice::from_raw_parts(buffer, bytes)}) {
            Ok(_) => FLAC__STREAM_ENCODER_WRITE_STATUS_OK,
            Err(e) => {
                eprintln!("On `write_callback()`: {:?}", e);
                FLAC__STREAM_ENCODER_WRITE_STATUS_FATAL_ERROR
            },
        }
    }

    unsafe extern "C" fn seek_callback(_encoder: *const FLAC__StreamEncoder, absolute_byte_offset: u64, client_data: *mut c_void) -> u32 {
        #[cfg(debug_assertions)]
        if SHOW_CALLBACKS {println!("seek_callback({absolute_byte_offset})");}
        let this = unsafe {&mut *(client_data as *mut Self)};
        match (this.on_seek)(this.writer, absolute_byte_offset) {
            Ok(_) => FLAC__STREAM_ENCODER_SEEK_STATUS_OK,
            Err(e) => {
                match e.kind() {
                    ErrorKind::NotSeekable => FLAC__STREAM_ENCODER_SEEK_STATUS_UNSUPPORTED,
                    _ => FLAC__STREAM_ENCODER_SEEK_STATUS_ERROR,
                }
            },
        }
    }

    unsafe extern "C" fn tell_callback(_encoder: *const FLAC__StreamEncoder, absolute_byte_offset: *mut u64, client_data: *mut c_void) -> u32 {
        let this = unsafe {&mut *(client_data as *mut Self)};
        match (this.on_tell)(this.writer) {
            Ok(offset) => {
                #[cfg(debug_assertions)]
                if SHOW_CALLBACKS {println!("tell_callback() == {offset}");}
                unsafe {*absolute_byte_offset = offset};
                FLAC__STREAM_ENCODER_TELL_STATUS_OK
            },
            Err(e) => {
                match e.kind() {
                    ErrorKind::NotSeekable => FLAC__STREAM_ENCODER_TELL_STATUS_UNSUPPORTED,
                    _ => FLAC__STREAM_ENCODER_TELL_STATUS_ERROR,
                }
            },
        }
    }

    unsafe extern "C" fn metadata_callback(_encoder: *const FLAC__StreamEncoder, metadata: *const FLAC__StreamMetadata, client_data: *mut c_void) {
        let _this = unsafe {&mut *(client_data as *mut Self)};
        let _meta = unsafe {*metadata};
        #[cfg(debug_assertions)]
        if SHOW_CALLBACKS {println!("{:?}", WrappedStreamMetadata(_meta))}
    }

    pub fn tell(&mut self) -> Result<u64, io::Error> {
        (self.on_tell)(self.writer)
    }

    pub fn write_interleaved_samples(&mut self, samples: &[i32]) -> Result<(), FlacEncoderError> {
        #[cfg(debug_assertions)]
        if SHOW_CALLBACKS {println!("write_interleaved_samples([i32; {}])", samples.len());}
        if samples.is_empty() {return Ok(())}
        if samples.len() % self.params.channels as usize != 0 {
            Err(FlacEncoderError::new(FLAC__STREAM_ENCODER_FRAMING_ERROR, "FlacEncoderUnmovable::write_interleaved_samples"))
        } else {
            unsafe {
                if FLAC__stream_encoder_process_interleaved(self.encoder, samples.as_ptr(), samples.len() as u32 / self.params.channels as u32) == 0 {
                    return self.get_status_as_error("FLAC__stream_encoder_process_interleaved");
                }
            }
            Ok(())
        }
    }

    pub fn write_mono_channel(&mut self, monos: &[i32]) -> Result<(), FlacEncoderError> {
        #[cfg(debug_assertions)]
        if SHOW_CALLBACKS {println!("write_mono_channel([i32; {}])", monos.len());}
        if monos.is_empty() {return Ok(())}
        match self.params.channels {
            1 => unsafe {
                if FLAC__stream_encoder_process_interleaved(self.encoder, monos.as_ptr(), monos.len() as u32) == 0 {
                    return self.get_status_as_error("FLAC__stream_encoder_process_interleaved");
                }
                Ok(())
            },
            2 => self.write_stereos(&monos.iter().map(|mono| -> (i32, i32){(*mono, *mono)}).collect::<Vec<(i32, i32)>>()),
            o => self.write_frames(&monos.iter().map(|mono| -> Vec<i32> {(0..o).map(|_|{*mono}).collect()}).collect::<Vec<Vec<i32>>>()),
        }
    }

    pub fn write_stereos(&mut self, stereos: &[(i32, i32)]) -> Result<(), FlacEncoderError> {
        #[cfg(debug_assertions)]
        if SHOW_CALLBACKS {println!("write_stereos([(i32, i32); {}])", stereos.len());}
        if stereos.is_empty() {return Ok(())}
        match self.params.channels {
            1 => self.write_mono_channel(&stereos.iter().map(|(l, r): &(i32, i32)| -> i32 {((*l as i64 + *r as i64) / 2) as i32}).collect::<Vec<i32>>()),
            2 => unsafe {
                let samples: Vec<i32> = stereos.iter().flat_map(|(l, r): &(i32, i32)| -> [i32; 2] {[*l, *r]}).collect();
                if FLAC__stream_encoder_process_interleaved(self.encoder, samples.as_ptr(), stereos.len() as u32) == 0 {
                    return self.get_status_as_error("FLAC__stream_encoder_process_interleaved");
                }
                Ok(())
            },
            o => panic!("Can't turn stereo audio into {o} channels audio."),
        }
    }

    pub fn write_monos(&mut self, monos: &[Vec<i32>]) -> Result<(), FlacEncoderError> {
        #[cfg(debug_assertions)]
        if SHOW_CALLBACKS {println!("write_monos([Vec<i32>; {}])", monos.len());}
        if monos.len() != self.params.channels as usize {
            Err(FlacEncoderError::new(FLAC__STREAM_ENCODER_FRAMING_ERROR, "FlacEncoderUnmovable::write_monos"))
        } else {
            unsafe {
                let len = monos[0].len();
                for mono in monos.iter() {
                    if mono.len() != len {
                        return Err(FlacEncoderError::new(FLAC__STREAM_ENCODER_FRAMING_ERROR, "FlacEncoderUnmovable::write_monos"));
                    }
                }
                let ptr_arr: Vec<*const i32> = monos.iter().map(|v|{v.as_ptr()}).collect();
                if FLAC__stream_encoder_process(self.encoder, ptr_arr.as_ptr(), len as u32) == 0 {
                    self.get_status_as_error("FLAC__stream_encoder_process")
                } else {
                    Ok(())
                }
            }
        }
    }

    pub fn write_frames(&mut self, frames: &[Vec<i32>]) -> Result<(), FlacEncoderError> {
        #[cfg(debug_assertions)]
        if SHOW_CALLBACKS {println!("write_frames([Vec<i32>; {}])", frames.len());}
        if frames.is_empty() {return Ok(())}
        let samples: Vec<i32> = frames.iter().flat_map(|frame: &Vec<i32>| -> Vec<i32> {
            if frame.len() != self.params.channels as usize {
                panic!("On FlacEncoderUnmovable::write_frames(): a frame size {} does not match the encoder channels.", frame.len())
            } else {frame.to_vec()}
        }).collect();
        unsafe {
            if FLAC__stream_encoder_process_interleaved(self.encoder, samples.as_ptr(), frames.len() as u32) == 0 {
                return self.get_status_as_error("FLAC__stream_encoder_process_interleaved");
            }
        }
        Ok(())
    }

    pub fn finish(&mut self) -> Result<(), FlacEncoderError> {
        if self.finished {
            return Ok(())
        }
        #[cfg(debug_assertions)]
        if SHOW_CALLBACKS {println!("finish()");}
        unsafe {
            if FLAC__stream_encoder_finish(self.encoder) != 0 {
                match self.writer.seek(SeekFrom::End(0)) {
                    Ok(_) => {self.finished = true; Ok(())},
                    Err(_) => Err(FlacEncoderError::new(FLAC__STREAM_ENCODER_IO_ERROR, "self.writer.seek(SeekFrom::End(0))")),
                }
            } else {
                self.get_status_as_error("FLAC__stream_encoder_finish")
            }
        }
    }

    fn on_drop(&mut self) {
        unsafe {
            if let Err(e) = self.finish() {
                eprintln!("On FlacEncoderUnmovable::finish(): {:?}", e);
            }

            self.metadata.clear();
            FLAC__stream_encoder_delete(self.encoder);
        };
    }

    pub fn finalize(self) {}
}

impl<'a, WriteSeek> Debug for FlacEncoderUnmovable<'_, WriteSeek>
where
    WriteSeek: Write + Seek + Debug {
    fn fmt(&self, fmt: &mut Formatter) -> fmt::Result {
        fmt.debug_struct("FlacEncoderUnmovable")
            .field("encoder", &self.encoder)
            .field("params", &self.params)
            .field("writer", &self.writer)
            .field("on_write", &"{{closure}}")
            .field("on_seek", &"{{closure}}")
            .field("on_tell", &"{{closure}}")
            .field("comments", &self.comments)
            .field("cue_sheets", &self.cue_sheets)
            .field("picture", &format_args!("..."))
            .finish()
    }
}

impl<'a, WriteSeek> Drop for FlacEncoderUnmovable<'_, WriteSeek>
where
    WriteSeek: Write + Seek + Debug {
    fn drop(&mut self) {
        self.on_drop();
    }
}

pub struct FlacEncoder<'a, WriteSeek>
where
    WriteSeek: Write + Seek + Debug {
    encoder: Box<FlacEncoderUnmovable<'a, WriteSeek>>,
}

impl<'a, WriteSeek> FlacEncoder<'a, WriteSeek>
where
    WriteSeek: Write + Seek + Debug {
    pub fn new(
        writer: &'a mut WriteSeek,
        on_write: Box<dyn FnMut(&mut WriteSeek, &[u8]) -> Result<(), io::Error> + 'a>,
        on_seek: Box<dyn FnMut(&mut WriteSeek, u64) -> Result<(), io::Error> + 'a>,
        on_tell: Box<dyn FnMut(&mut WriteSeek) -> Result<u64, io::Error> + 'a>,
        params: &FlacEncoderParams
    ) -> Result<Self, FlacEncoderError> {
        Ok(Self {
            encoder: Box::new(FlacEncoderUnmovable::new(writer, on_write, on_seek, on_tell, params)?)
        })
    }

    pub fn insert_comments(&mut self, key: &'static str, value: &str) -> Result<(), FlacEncoderInitError> {
        self.encoder.insert_comments(key, value)
    }

    pub fn insert_cue_sheet(&mut self, cue_sheet: &FlacCueSheet) -> Result<(), FlacEncoderInitError> {
        self.encoder.insert_cue_sheet(cue_sheet)
    }

    pub fn add_picture(&mut self, picture_binary: &[u8], description: &str, mime_type: &str, width: u32, height: u32, depth: u32, colors: u32) -> Result<(), FlacEncoderInitError> {
        self.encoder.add_picture(picture_binary, description, mime_type, width, height, depth, colors)
    }

    #[cfg(feature = "id3")]
    pub fn inherit_metadata_from_id3(&mut self, tag: &id3::Tag) -> Result<(), FlacEncoderInitError> {
        self.encoder.inherit_metadata_from_id3(tag)
    }

    pub fn get_params(&self) -> FlacEncoderParams {
        self.encoder.get_params()
    }

    pub fn tell(&mut self) -> Result<u64, io::Error> {
        self.encoder.tell()
    }

    pub fn initialize(&mut self) -> Result<(), FlacEncoderInitError> {
        if !self.encoder.encoder_initialized {
            self.encoder.initialize()?
        }
        Ok(())
    }

    pub fn write_interleaved_samples(&mut self, samples: &[i32]) -> Result<(), FlacEncoderError> {
        self.encoder.write_interleaved_samples(samples)
    }

    pub fn write_mono_channel(&mut self, monos: &[i32]) -> Result<(), FlacEncoderError> {
        self.encoder.write_mono_channel(monos)
    }

    pub fn write_stereos(&mut self, stereos: &[(i32, i32)]) -> Result<(), FlacEncoderError> {
        self.encoder.write_stereos(stereos)
    }

    pub fn write_monos(&mut self, monos: &[Vec<i32>]) -> Result<(), FlacEncoderError> {
        self.encoder.write_monos(monos)
    }

    pub fn write_frames(&mut self, frames: &[Vec<i32>]) -> Result<(), FlacEncoderError> {
        self.encoder.write_frames(frames)
    }

    pub fn finish(&mut self) -> Result<(), FlacEncoderError> {
        self.encoder.finish()
    }

    pub fn finalize(self) {}
}

impl<'a, WriteSeek> Debug for FlacEncoder<'_, WriteSeek>
where
    WriteSeek: Write + Seek + Debug {
    fn fmt(&self, fmt: &mut Formatter) -> fmt::Result {
        fmt.debug_struct("FlacEncoder")
            .field("encoder", &self.encoder)
            .finish()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FlacDecoderError {
    pub code: u32,
    pub message: &'static str,
    pub function: &'static str,
}

impl FlacDecoderError {
    pub fn new(code: u32, function: &'static str) -> Self {
        Self {
            code,
            message: Self::get_message_from_code(code),
            function,
        }
    }

    pub fn get_message_from_code(code: u32) -> &'static str {
        unsafe {
            CStr::from_ptr(*FLAC__StreamDecoderStateString.as_ptr().add(code as usize)).to_str().unwrap()
        }
    }
}

impl_FlacError!(FlacDecoderError);

#[derive(Debug, Clone, Copy)]
pub enum FlacDecoderErrorCode {
    StreamDecoderSearchForMetadata = FLAC__STREAM_DECODER_SEARCH_FOR_METADATA as isize,
    StreamDecoderReadMetadata = FLAC__STREAM_DECODER_READ_METADATA as isize,
    StreamDecoderSearchForFrameSync = FLAC__STREAM_DECODER_SEARCH_FOR_FRAME_SYNC as isize,
    StreamDecoderReadFrame = FLAC__STREAM_DECODER_READ_FRAME as isize,
    StreamDecoderEndOfStream = FLAC__STREAM_DECODER_END_OF_STREAM as isize,
    StreamDecoderOggError = FLAC__STREAM_DECODER_OGG_ERROR as isize,
    StreamDecoderSeekError = FLAC__STREAM_DECODER_SEEK_ERROR as isize,
    StreamDecoderAborted = FLAC__STREAM_DECODER_ABORTED as isize,
    StreamDecoderMemoryAllocationError = FLAC__STREAM_DECODER_MEMORY_ALLOCATION_ERROR as isize,
    StreamDecoderUninitialized = FLAC__STREAM_DECODER_UNINITIALIZED as isize,
}

impl Display for FlacDecoderErrorCode {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Self::StreamDecoderSearchForMetadata => write!(f, "The decoder is ready to search for metadata."),
            Self::StreamDecoderReadMetadata => write!(f, "The decoder is ready to or is in the process of reading metadata."),
            Self::StreamDecoderSearchForFrameSync => write!(f, "The decoder is ready to or is in the process of searching for the frame sync code."),
            Self::StreamDecoderReadFrame => write!(f, "The decoder is ready to or is in the process of reading a frame."),
            Self::StreamDecoderEndOfStream => write!(f, "The decoder has reached the end of the stream."),
            Self::StreamDecoderOggError => write!(f, "An error occurred in the underlying Ogg layer."),
            Self::StreamDecoderSeekError => write!(f, "An error occurred while seeking. The decoder must be flushed with FLAC__stream_decoder_flush() or reset with FLAC__stream_decoder_reset() before decoding can continue."),
            Self::StreamDecoderAborted => write!(f, "The decoder was aborted by the read or write callback."),
            Self::StreamDecoderMemoryAllocationError => write!(f, "An error occurred allocating memory. The decoder is in an invalid state and can no longer be used."),
            Self::StreamDecoderUninitialized => write!(f, "The decoder is in the uninitialized state; one of the FLAC__stream_decoder_init_*() functions must be called before samples can be processed."),
        }
    }
}

impl From<u32> for FlacDecoderErrorCode {
    fn from(code: u32) -> Self {
        use FlacDecoderErrorCode::*;
        match code {
            FLAC__STREAM_DECODER_SEARCH_FOR_METADATA => StreamDecoderSearchForMetadata,
            FLAC__STREAM_DECODER_READ_METADATA => StreamDecoderReadMetadata,
            FLAC__STREAM_DECODER_SEARCH_FOR_FRAME_SYNC => StreamDecoderSearchForFrameSync,
            FLAC__STREAM_DECODER_READ_FRAME => StreamDecoderReadFrame,
            FLAC__STREAM_DECODER_END_OF_STREAM => StreamDecoderEndOfStream,
            FLAC__STREAM_DECODER_OGG_ERROR => StreamDecoderOggError,
            FLAC__STREAM_DECODER_SEEK_ERROR => StreamDecoderSeekError,
            FLAC__STREAM_DECODER_ABORTED => StreamDecoderAborted,
            FLAC__STREAM_DECODER_MEMORY_ALLOCATION_ERROR => StreamDecoderMemoryAllocationError,
            FLAC__STREAM_DECODER_UNINITIALIZED => StreamDecoderUninitialized,
            o => panic!("Not an decoder error code: {o}."),
        }
    }
}

impl std::error::Error for FlacDecoderErrorCode {}

#[derive(Debug, Clone, Copy)]
pub struct FlacDecoderInitError {
    pub code: u32,
    pub message: &'static str,
    pub function: &'static str,
}

impl FlacDecoderInitError {
    pub fn new(code: u32, function: &'static str) -> Self {
        Self {
            code,
            message: Self::get_message_from_code(code),
            function,
        }
    }

    pub fn get_message_from_code(code: u32) -> &'static str {
        unsafe {
            CStr::from_ptr(*FLAC__StreamDecoderInitStatusString.as_ptr().add(code as usize)).to_str().unwrap()
        }
    }
}

impl_FlacError!(FlacDecoderInitError);

#[derive(Debug, Clone, Copy)]
pub enum FlacDecoderInitErrorCode {
    StreamDecoderInitStatusOk = FLAC__STREAM_DECODER_INIT_STATUS_OK as isize,
    StreamDecoderInitStatusUnsupportedContainer = FLAC__STREAM_DECODER_INIT_STATUS_UNSUPPORTED_CONTAINER as isize,
    StreamDecoderInitStatusInvalidCallbacks = FLAC__STREAM_DECODER_INIT_STATUS_INVALID_CALLBACKS as isize,
    StreamDecoderInitStatusMemoryAllocationError = FLAC__STREAM_DECODER_INIT_STATUS_MEMORY_ALLOCATION_ERROR as isize,
    StreamDecoderInitStatusErrorOpeningFile = FLAC__STREAM_DECODER_INIT_STATUS_ERROR_OPENING_FILE as isize,
    StreamDecoderInitStatusAlreadyInitialized = FLAC__STREAM_DECODER_INIT_STATUS_ALREADY_INITIALIZED as isize,
}

impl Display for FlacDecoderInitErrorCode {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Self::StreamDecoderInitStatusOk => write!(f, "Initialization was successful."),
            Self::StreamDecoderInitStatusUnsupportedContainer => write!(f, "The library was not compiled with support for the given container format."),
            Self::StreamDecoderInitStatusInvalidCallbacks => write!(f, "A required callback was not supplied."),
            Self::StreamDecoderInitStatusMemoryAllocationError => write!(f, "An error occurred allocating memory."),
            Self::StreamDecoderInitStatusErrorOpeningFile => write!(f, "fopen() failed in FLAC__stream_decoder_init_file() or FLAC__stream_decoder_init_ogg_file()."),
            Self::StreamDecoderInitStatusAlreadyInitialized => write!(f, "FLAC__stream_decoder_init_*() was called when the decoder was already initialized, usually because FLAC__stream_decoder_finish() was not called."),
        }
    }
}

impl From<u32> for FlacDecoderInitErrorCode {
    fn from(code: u32) -> Self {
        use FlacDecoderInitErrorCode::*;
        match code {
            FLAC__STREAM_DECODER_INIT_STATUS_OK => StreamDecoderInitStatusOk,
            FLAC__STREAM_DECODER_INIT_STATUS_UNSUPPORTED_CONTAINER => StreamDecoderInitStatusUnsupportedContainer,
            FLAC__STREAM_DECODER_INIT_STATUS_INVALID_CALLBACKS => StreamDecoderInitStatusInvalidCallbacks,
            FLAC__STREAM_DECODER_INIT_STATUS_MEMORY_ALLOCATION_ERROR => StreamDecoderInitStatusMemoryAllocationError,
            FLAC__STREAM_DECODER_INIT_STATUS_ERROR_OPENING_FILE => StreamDecoderInitStatusErrorOpeningFile,
            FLAC__STREAM_DECODER_INIT_STATUS_ALREADY_INITIALIZED => StreamDecoderInitStatusAlreadyInitialized,
            o => panic!("Not an decoder init error code: {o}."),
        }
    }
}

impl std::error::Error for FlacDecoderInitErrorCode {}

impl From<FlacDecoderError> for FlacDecoderInitError {
    fn from(err: FlacDecoderError) -> Self {
        Self {
            code: err.code,
            message: err.message,
            function: err.function,
        }
    }
}

impl From<FlacDecoderInitError> for FlacDecoderError {
    fn from(err: FlacDecoderInitError) -> Self {
        Self {
            code: err.code,
            message: err.message,
            function: err.function,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum FlacReadStatus {
    GoOn,
    Eof,
    Abort,
}

impl Display for FlacReadStatus {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Self::GoOn => write!(f, "go_on"),
            Self::Eof => write!(f, "eof"),
            Self::Abort => write!(f, "abort"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum FlacInternalDecoderError {
    LostSync,
    BadHeader,
    FrameCrcMismatch,
    UnparseableStream,
    BadMetadata,
    OutOfBounds,
}

impl Display for FlacInternalDecoderError {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Self::LostSync => write!(f, "An error in the stream caused the decoder to lose synchronization."),
            Self::BadHeader => write!(f, "The decoder encountered a corrupted frame header."),
            Self::FrameCrcMismatch => write!(f, "The frame's data did not match the CRC in the footer."),
            Self::UnparseableStream => write!(f, "The decoder encountered reserved fields in use in the stream."),
            Self::BadMetadata => write!(f, "The decoder encountered a corrupted metadata block."),
            Self::OutOfBounds => write!(f, "The decoder encountered a otherwise valid frame in which the decoded samples exceeded the range offered by the stated bit depth."),
        }
    }
}

impl std::error::Error for FlacInternalDecoderError {}

#[derive(Debug, Clone, Copy)]
pub enum FlacAudioForm {
    FrameArray,
    ChannelArray,
}

#[derive(Debug, Clone, Copy)]
pub struct SamplesInfo {
    pub samples: u32,
    pub channels: u32,
    pub sample_rate: u32,
    pub bits_per_sample: u32,
    pub audio_form: FlacAudioForm,
}

fn entry_to_str(entry: &FLAC__StreamMetadata_VorbisComment_Entry) -> Cow<'_, str> {
    unsafe{String::from_utf8_lossy(slice::from_raw_parts(entry.entry, entry.length as usize))}
}

fn entry_to_string(entry: &FLAC__StreamMetadata_VorbisComment_Entry) -> String {
    entry_to_str(entry).to_string()
}

pub struct FlacDecoderUnmovable<'a, ReadSeek>
where
    ReadSeek: Read + Seek + Debug {
    // https://xiph.org/flac/api/group__flac__stream__decoder.html
    decoder: *mut FLAC__StreamDecoder,
    reader: &'a mut ReadSeek,
    on_read: Box<dyn FnMut(&mut ReadSeek, &mut [u8]) -> (usize, FlacReadStatus) + 'a>,
    on_seek: Box<dyn FnMut(&mut ReadSeek, u64) -> Result<(), io::Error> + 'a>,
    on_tell: Box<dyn FnMut(&mut ReadSeek) -> Result<u64, io::Error> + 'a>,
    on_length: Box<dyn FnMut(&mut ReadSeek) -> Result<u64, io::Error> + 'a>,
    on_eof: Box<dyn FnMut(&mut ReadSeek) -> bool + 'a>,
    on_write: Box<dyn FnMut(&[Vec<i32>], &SamplesInfo) -> Result<(), io::Error> + 'a>,
    on_error: Box<dyn FnMut(FlacInternalDecoderError) + 'a>,
    md5_checking: bool,
    finished: bool,
    pub scale_to_i32_range: bool,
    pub desired_audio_form: FlacAudioForm,
    pub vendor_string: Option<String>,
    pub comments: BTreeMap<String, String>,
    pub pictures: Vec<PictureData>,
    pub cue_sheets: Vec<FlacCueSheet>,
}

impl<'a, ReadSeek> FlacDecoderUnmovable<'a, ReadSeek>
where
    ReadSeek: Read + Seek + Debug {
    pub fn new(
        reader: &'a mut ReadSeek,
        on_read: Box<dyn FnMut(&mut ReadSeek, &mut [u8]) -> (usize, FlacReadStatus) + 'a>,
        on_seek: Box<dyn FnMut(&mut ReadSeek, u64) -> Result<(), io::Error> + 'a>,
        on_tell: Box<dyn FnMut(&mut ReadSeek) -> Result<u64, io::Error> + 'a>,
        on_length: Box<dyn FnMut(&mut ReadSeek) -> Result<u64, io::Error> + 'a>,
        on_eof: Box<dyn FnMut(&mut ReadSeek) -> bool + 'a>,
        on_write: Box<dyn FnMut(&[Vec<i32>], &SamplesInfo) -> Result<(), io::Error> + 'a>,
        on_error: Box<dyn FnMut(FlacInternalDecoderError) + 'a>,
        md5_checking: bool,
        scale_to_i32_range: bool,
        desired_audio_form: FlacAudioForm,
    ) -> Result<Self, FlacDecoderError> {
        let ret = Self {
            decoder: unsafe {FLAC__stream_decoder_new()},
            reader,
            on_read,
            on_seek,
            on_tell,
            on_length,
            on_eof,
            on_write,
            on_error,
            md5_checking,
            finished: false,
            scale_to_i32_range,
            desired_audio_form,
            vendor_string: None,
            comments: BTreeMap::new(),
            pictures: Vec::<PictureData>::new(),
            cue_sheets: Vec::<FlacCueSheet>::new(),
        };
        if ret.decoder.is_null() {
            Err(FlacDecoderError::new(FLAC__STREAM_DECODER_MEMORY_ALLOCATION_ERROR, "FLAC__stream_decoder_new"))
        } else {
            Ok(ret)
        }
    }

    fn get_status_as_result(&self, function: &'static str) -> Result<(), FlacDecoderError> {
        let code = unsafe {FLAC__stream_decoder_get_state(self.decoder)};
        if code == 0 {
            Ok(())
        } else {
            Err(FlacDecoderError::new(code, function))
        }
    }

    fn get_status_as_error(&self, function: &'static str) -> Result<(), FlacDecoderError> {
        let code = unsafe {FLAC__stream_decoder_get_state(self.decoder)};
        Err(FlacDecoderError::new(code, function))
    }

    fn as_ptr(&self) -> *const Self {
        self as *const Self
    }

    fn as_mut_ptr(&mut self) -> *mut Self {
        self as *mut Self
    }

    unsafe extern "C" fn read_callback(_decoder: *const FLAC__StreamDecoder, buffer: *mut u8, bytes: *mut usize, client_data: *mut c_void) -> u32 {
        let this = unsafe {&mut *(client_data as *mut Self)};
        if unsafe {*bytes} == 0 {
            FLAC__STREAM_DECODER_READ_STATUS_ABORT
        } else {
            let buf = unsafe {slice::from_raw_parts_mut(buffer, *bytes)};
            let (bytes_read, status) = (this.on_read)(this.reader, buf);
            let ret = match status{
                FlacReadStatus::GoOn => FLAC__STREAM_DECODER_READ_STATUS_CONTINUE,
                FlacReadStatus::Eof => FLAC__STREAM_DECODER_READ_STATUS_END_OF_STREAM,
                FlacReadStatus::Abort => FLAC__STREAM_DECODER_READ_STATUS_ABORT,
            };

            unsafe {*bytes = bytes_read};
            ret
        }
    }

    unsafe extern "C" fn seek_callback(_decoder: *const FLAC__StreamDecoder, absolute_byte_offset: u64, client_data: *mut c_void) -> u32 {
        let this = unsafe {&mut *(client_data as *mut Self)};
        match (this.on_seek)(this.reader, absolute_byte_offset) {
            Ok(_) => FLAC__STREAM_DECODER_SEEK_STATUS_OK,
            Err(e) => {
                match e.kind() {
                    ErrorKind::NotSeekable => FLAC__STREAM_DECODER_SEEK_STATUS_UNSUPPORTED,
                    _ => FLAC__STREAM_DECODER_SEEK_STATUS_ERROR,
                }
            },
        }
    }

    unsafe extern "C" fn tell_callback(_decoder: *const FLAC__StreamDecoder, absolute_byte_offset: *mut u64, client_data: *mut c_void) -> u32 {
        let this = unsafe {&mut *(client_data as *mut Self)};
        match (this.on_tell)(this.reader) {
            Ok(offset) => {
                unsafe {*absolute_byte_offset = offset};
                FLAC__STREAM_DECODER_TELL_STATUS_OK
            },
            Err(e) => {
                match e.kind() {
                    ErrorKind::NotSeekable => FLAC__STREAM_DECODER_TELL_STATUS_UNSUPPORTED,
                    _ => FLAC__STREAM_DECODER_TELL_STATUS_ERROR,
                }
            },
        }
    }

    unsafe extern "C" fn length_callback(_decoder: *const FLAC__StreamDecoder, stream_length: *mut u64, client_data: *mut c_void) -> u32 {
        let this = unsafe {&mut *(client_data as *mut Self)};
        match (this.on_length)(this.reader) {
            Ok(length) => {
                unsafe {*stream_length = length};
                FLAC__STREAM_DECODER_LENGTH_STATUS_OK
            },
            Err(e) => {
                match e.kind() {
                    ErrorKind::NotSeekable => FLAC__STREAM_DECODER_LENGTH_STATUS_UNSUPPORTED,
                    _ => FLAC__STREAM_DECODER_LENGTH_STATUS_ERROR,
                }
            },
        }
    }

    unsafe extern "C" fn eof_callback(_decoder: *const FLAC__StreamDecoder, client_data: *mut c_void) -> i32 {
        let this = unsafe {&mut *(client_data as *mut Self)};
        if (this.on_eof)(this.reader) {1} else {0}
    }

    unsafe extern "C" fn write_callback(_decoder: *const FLAC__StreamDecoder, frame: *const FLAC__Frame, buffer: *const *const i32, client_data: *mut c_void) -> u32 {
        // Scales signed PCM samples to full i32 dynamic range.
        // - `bits`: Valid bits in `sample` (1-32).
        // - Example: 8-bit samples [-128, 127] → [i32::MIN, i32::MAX]
        fn scale_to_i32(sample: i32, bits: u32) -> i32 {
            assert!(bits <= 32);
            if bits == 32 {
                sample
            } else {
                fn scale_to_unsigned(sample: i32, bits: u32) -> u32 {
                    let mask = (1u32 << bits) - 1;
                    let mid_number = 1u32 << (bits - 1);
                    ((sample as u32).wrapping_add(mid_number) & mask) << (32 - bits)
                }
                let mut lower_fill = scale_to_unsigned(sample, bits);
                let mut result = (sample as u32) << (32 - bits);
                while lower_fill > 0 {
                    lower_fill >>= bits;
                    result |= lower_fill;
                }
                result as i32
            }
        }

        let this = unsafe {&mut *(client_data as *mut Self)};
        let frame = unsafe {*frame};
        let samples = frame.header.blocksize;
        let channels = frame.header.channels;
        let sample_rate = frame.header.sample_rate;
        let bits_per_sample = frame.header.bits_per_sample;

        let mut samples_info = SamplesInfo {
            samples,
            channels,
            sample_rate,
            bits_per_sample,
            audio_form: this.desired_audio_form,
        };

        let mut ret: Vec<Vec<i32>>;
        match this.desired_audio_form {
            FlacAudioForm::FrameArray => {
                // Each `frame` contains one sample for each channel
                ret = vec![Vec::<i32>::new(); samples as usize];
                for s in 0..samples {
                    for c in 0..channels {
                        let channel = unsafe {*buffer.add(c as usize)};
                        ret[s as usize].push(unsafe {*channel.add(s as usize)});
                    }
                }
            },
            FlacAudioForm::ChannelArray => {
                // Each `channel` contains all samples for the channel
                ret = vec![Vec::<i32>::new(); channels as usize];
                for c in 0..channels {
                    ret[c as usize] = unsafe {slice::from_raw_parts(*buffer.add(c as usize), samples as usize)}.to_vec();
                }
            }
        }

        // Whatever it was, now it's just a two-dimensional array
        if this.scale_to_i32_range {
            for x in ret.iter_mut() {
                for y in x.iter_mut() {
                    *y = scale_to_i32(*y, bits_per_sample);
                }
            }
            samples_info.bits_per_sample = 32;
        }

        match (this.on_write)(&ret, &samples_info) {
            Ok(_) => FLAC__STREAM_DECODER_WRITE_STATUS_CONTINUE,
            Err(e) => {
                eprintln!("On `write_callback()`: {:?}", e);
                FLAC__STREAM_DECODER_WRITE_STATUS_ABORT
            },
        }
    }

    unsafe extern "C" fn metadata_callback(_decoder: *const FLAC__StreamDecoder, metadata: *const FLAC__StreamMetadata, client_data: *mut c_void) {
        let this = unsafe {&mut *(client_data as *mut Self)};
        let metadata = unsafe {*metadata};
        match metadata.type_ {
            FLAC__METADATA_TYPE_VORBIS_COMMENT => unsafe {
                let comments = metadata.data.vorbis_comment;

                // First retrieve the vendor string
                this.vendor_string = Some(entry_to_string(&comments.vendor_string));

                // Then to get all of the key pairs, the key pairs should be all uppercase, but some of them are not.
                // Read both the uppercase keys and the lowercase keys and store them, if it won't overwrite then we convert
                // the key to uppercase and store it again.
                let mut uppercase_keypairs = Vec::<(String, String)>::new();
                for i in 0..comments.num_comments {
                    let comment = entry_to_string(&*comments.comments.add(i as usize));

                    // The key pair is split by the equal notation
                    let mut iter = comment.split("=");
                    if let Some(key) = iter.next() {
                        let key = key.to_owned();

                        // Ignore the later equal notations.
                        let val = iter.map(|s: &str|{s.to_string()}).collect::<Vec<String>>().join("=");
                        let key_upper = key.to_uppercase();
                        if key != key_upper {
                            uppercase_keypairs.push((key_upper, val.clone()));
                        }

                        // Duplication check
                        let if_dup = format!("Duplicated comments: new comment is {key}: {val}, the previous is {key}: ");
                        if let Some(old) = this.comments.insert(key, val) {
                            eprintln!("{if_dup}{old}");
                        }
                    } else {
                        // No equal notation here
                        eprintln!("Invalid comment: {comment}");
                    }
                }

                // If it lacks the uppercase key pairs, we add it to the map.
                for (key_upper, val) in uppercase_keypairs {
                    if this.comments.contains_key(&key_upper) {
                        continue;
                    } else {
                        this.comments.insert(key_upper, val);
                    }
                }
            },
            FLAC__METADATA_TYPE_PICTURE => unsafe {
                let picture = metadata.data.picture;
                this.pictures.push(PictureData{
                    picture: slice::from_raw_parts(picture.data, picture.data_length as usize).to_vec(),
                    description: CStr::from_ptr(picture.description as *const i8).to_string_lossy().to_string(),
                    mime_type: CStr::from_ptr(picture.mime_type).to_string_lossy().to_string(),
                    width: picture.width,
                    height: picture.height,
                    depth: picture.depth,
                    colors: picture.colors,
                });
            },
            FLAC__METADATA_TYPE_CUESHEET => unsafe {
                let cue_sheet = metadata.data.cue_sheet;
                this.cue_sheets.push(FlacCueSheet{
                    media_catalog_number: cue_sheet.media_catalog_number,
                    lead_in: cue_sheet.lead_in,
                    is_cd: cue_sheet.is_cd != 0,
                    tracks: (0..cue_sheet.num_tracks).map(|i| -> (u8, FlacCueTrack) {
                        let track = *cue_sheet.tracks.add(i as usize);
                        (track.number, FlacCueTrack {
                            offset: track.offset,
                            track_no: track.number,
                            isrc: track.isrc,
                            type_: match track.type_() {
                                0 => FlacTrackType::Audio,
                                _ => FlacTrackType::NonAudio,
                            },
                            pre_emphasis: track.pre_emphasis() != 0,
                            indices: (0..track.num_indices).map(|i| -> FlacCueSheetIndex {
                                let index = *track.indices.add(i as usize);
                                FlacCueSheetIndex {
                                    offset: index.offset,
                                    number: index.number,
                                }
                            }).collect()
                        })
                    }).collect(),
                });
            },
            _ => {
                #[cfg(debug_assertions)]
                if SHOW_CALLBACKS {println!("On `metadata_callback()`: {:?}", WrappedStreamMetadata(metadata));}
            },
        }
    }

    unsafe extern "C" fn error_callback(_decoder: *const FLAC__StreamDecoder, status: u32, client_data: *mut c_void) {
        let this = unsafe {&mut *(client_data as *mut Self)};
        (this.on_error)(match status {
            FLAC__STREAM_DECODER_ERROR_STATUS_LOST_SYNC => FlacInternalDecoderError::LostSync,
            FLAC__STREAM_DECODER_ERROR_STATUS_BAD_HEADER => FlacInternalDecoderError::BadHeader,
            FLAC__STREAM_DECODER_ERROR_STATUS_FRAME_CRC_MISMATCH => FlacInternalDecoderError::FrameCrcMismatch,
            FLAC__STREAM_DECODER_ERROR_STATUS_UNPARSEABLE_STREAM => FlacInternalDecoderError::UnparseableStream,
            FLAC__STREAM_DECODER_ERROR_STATUS_BAD_METADATA => FlacInternalDecoderError::BadMetadata,
            o => panic!("Unknown value of `FLAC__StreamDecodeErrorStatus`: {o}"),
        });
    }

    pub fn initialize(&mut self) -> Result<(), FlacDecoderError> {
        unsafe {
            if FLAC__stream_decoder_set_md5_checking(self.decoder, self.md5_checking as i32) == 0 {
                return self.get_status_as_error("FLAC__stream_decoder_set_md5_checking");
            }
            if FLAC__stream_decoder_set_metadata_respond_all(self.decoder) == 0 {
                return self.get_status_as_error("FLAC__stream_decoder_set_metadata_respond_all");
            }
            let ret = FLAC__stream_decoder_init_stream(
                self.decoder,
                Some(Self::read_callback),
                Some(Self::seek_callback),
                Some(Self::tell_callback),
                Some(Self::length_callback),
                Some(Self::eof_callback),
                Some(Self::write_callback),
                Some(Self::metadata_callback),
                Some(Self::error_callback),
                self.as_mut_ptr() as *mut c_void,
            );
            if ret != 0 {
                return Err(FlacDecoderError {
                    code: ret,
                    message: FlacDecoderInitError::get_message_from_code(ret),
                    function: "FLAC__stream_decoder_init_stream",
                });
            }
        }
        self.finished = false;
        self.get_status_as_result("FlacDecoderUnmovable::Init()")
    }

    pub fn seek(&mut self, frame_index: u64) -> Result<(), FlacDecoderError> {
        for _retry in 0..3 {
            unsafe {
                if FLAC__stream_decoder_seek_absolute(self.decoder, frame_index) == 0 {
                    match FLAC__stream_decoder_get_state(self.decoder) {
                        FLAC__STREAM_DECODER_SEEK_STATUS_OK => panic!("`FLAC__stream_decoder_seek_absolute()` returned false, but the status of the decoder is `OK`"),
                        FLAC__STREAM_DECODER_SEEK_ERROR => {
                            if FLAC__stream_decoder_reset(self.decoder) == 0 {
                                return self.get_status_as_error("FLAC__stream_decoder_reset");
                            } else {
                                continue;
                            }
                        },
                        o => return Err(FlacDecoderError::new(o, "FLAC__stream_decoder_seek_absolute")),
                    }
                } else {
                    return Ok(())
                }
            }
        }
        Err(FlacDecoderError::new(FLAC__STREAM_DECODER_SEEK_ERROR, "FLAC__stream_decoder_seek_absolute"))
    }

    pub fn tell(&mut self) -> Result<u64, io::Error> {
        (self.on_tell)(self.reader)
    }

    pub fn length(&mut self) -> Result<u64, io::Error> {
        (self.on_length)(self.reader)
    }

    pub fn eof(&mut self) -> bool {
        (self.on_eof)(self.reader)
    }

    pub fn get_comments(&self) -> &BTreeMap<String, String> {
        &self.comments
    }

    pub fn get_cue_sheets(&self) -> &Vec<FlacCueSheet> {
        &self.cue_sheets
    }

    pub fn decode(&mut self) -> Result<bool, FlacDecoderError> {
        if unsafe {FLAC__stream_decoder_process_single(self.decoder) != 0} {
            Ok(true)
        } else {
            match self.get_status_as_result("FLAC__stream_decoder_process_single") {
                Ok(_) => Ok(false),
                Err(e) => Err(e),
            }
        }
    }

    pub fn decode_all(&mut self) -> Result<bool, FlacDecoderError> {
        if unsafe {FLAC__stream_decoder_process_until_end_of_stream(self.decoder) != 0} {
            Ok(true)
        } else {
            match self.get_status_as_result("FLAC__stream_decoder_process_until_end_of_stream") {
                Ok(_) => Ok(false),
                Err(e) => Err(e),
            }
        }
    }

    pub fn finish(&mut self) -> Result<(), FlacDecoderError> {
        if !self.finished {
            if unsafe {FLAC__stream_decoder_finish(self.decoder) != 0} {
                self.finished = true;
                Ok(())
            } else {
                self.get_status_as_result("FLAC__stream_decoder_finish")
            }
        } else {
            Ok(())
        }
    }

    fn on_drop(&mut self) {
        unsafe {
            if let Err(e) =  self.finish() {
                eprintln!("On FlacDecoderUnmovable::finish(): {:?}", e);
            }

            // Must delete `self.decoder` even `self.finish()` fails.
            FLAC__stream_decoder_delete(self.decoder);
        };
    }

    pub fn finalize(self) {}
}

impl<'a, ReadSeek> Debug for FlacDecoderUnmovable<'_, ReadSeek>
where
    ReadSeek: Read + Seek + Debug {
    fn fmt(&self, fmt: &mut Formatter) -> fmt::Result {
        fmt.debug_struct("FlacDecoderUnmovable")
            .field("decoder", &self.decoder)
            .field("reader", &self.reader)
            .field("on_read", &"{{closure}}")
            .field("on_seek", &"{{closure}}")
            .field("on_tell", &"{{closure}}")
            .field("on_length", &"{{closure}}")
            .field("on_eof", &"{{closure}}")
            .field("on_write", &"{{closure}}")
            .field("on_error", &"{{closure}}")
            .field("md5_checking", &self.md5_checking)
            .finish()
    }
}

impl<'a, ReadSeek> Drop for FlacDecoderUnmovable<'_, ReadSeek>
where
    ReadSeek: Read + Seek + Debug {
    fn drop(&mut self) {
        self.on_drop();
    }
}

pub struct FlacDecoder<'a, ReadSeek>
where
    ReadSeek: Read + Seek + Debug {
    decoder: Box<FlacDecoderUnmovable<'a, ReadSeek>>,
}

impl<'a, ReadSeek> FlacDecoder<'a, ReadSeek>
where
    ReadSeek: Read + Seek + Debug {
    pub fn new(
        reader: &'a mut ReadSeek,
        on_read: Box<dyn FnMut(&mut ReadSeek, &mut [u8]) -> (usize, FlacReadStatus) + 'a>,
        on_seek: Box<dyn FnMut(&mut ReadSeek, u64) -> Result<(), io::Error> + 'a>,
        on_tell: Box<dyn FnMut(&mut ReadSeek) -> Result<u64, io::Error> + 'a>,
        on_length: Box<dyn FnMut(&mut ReadSeek) -> Result<u64, io::Error> + 'a>,
        on_eof: Box<dyn FnMut(&mut ReadSeek) -> bool + 'a>,
        on_write: Box<dyn FnMut(&[Vec<i32>], &SamplesInfo) -> Result<(), io::Error> + 'a>,
        on_error: Box<dyn FnMut(FlacInternalDecoderError) + 'a>,
        md5_checking: bool,
        scale_to_i32_range: bool,
        desired_audio_form: FlacAudioForm,
    ) -> Result<Self, FlacDecoderError> {
        let mut ret = Self {
            decoder: Box::new(FlacDecoderUnmovable::<'a>::new(
                reader,
                on_read,
                on_seek,
                on_tell,
                on_length,
                on_eof,
                on_write,
                on_error,
                md5_checking,
                scale_to_i32_range,
                desired_audio_form,
            )?),
        };
        ret.decoder.initialize()?;
        Ok(ret)
    }

    pub fn seek(&mut self, frame_index: u64) -> Result<(), FlacDecoderError> {
        self.decoder.seek(frame_index)
    }

    pub fn tell(&mut self) -> Result<u64, io::Error> {
        self.decoder.tell()
    }

    pub fn length(&mut self) -> Result<u64, io::Error> {
        self.decoder.length()
    }

    pub fn eof(&mut self) -> bool {
        self.decoder.eof()
    }

    pub fn get_vendor_string(&self) -> &Option<String> {
        &self.decoder.vendor_string
    }

    pub fn get_comments(&self) -> &BTreeMap<String, String> {
        &self.decoder.comments
    }

    pub fn get_pictures(&self) -> &Vec<PictureData> {
        &self.decoder.pictures
    }

    pub fn decode(&mut self) -> Result<bool, FlacDecoderError> {
        self.decoder.decode()
    }

    pub fn decode_all(&mut self) -> Result<bool, FlacDecoderError> {
        self.decoder.decode_all()
    }

    pub fn finish(&mut self) -> Result<(), FlacDecoderError> {
        self.decoder.finish()
    }

    pub fn finalize(self) {}
}

impl<'a, ReadSeek> Debug for FlacDecoder<'_, ReadSeek>
where
    ReadSeek: Read + Seek + Debug {
    fn fmt(&self, fmt: &mut Formatter) -> fmt::Result {
        fmt.debug_struct("FlacDecoder")
            .field("decoder", &self.decoder)
            .finish()
    }
}

#[derive(Clone, Copy)]
struct WrappedStreamInfo(FLAC__StreamMetadata_StreamInfo);

impl Debug for WrappedStreamInfo {
    fn fmt(&self, fmt: &mut Formatter) -> fmt::Result {
        fmt.debug_struct("FLAC__StreamMetadata_StreamInfo")
            .field("min_blocksize", &self.0.min_blocksize)
            .field("max_blocksize", &self.0.max_blocksize)
            .field("min_framesize", &self.0.min_framesize)
            .field("max_framesize", &self.0.max_framesize)
            .field("sample_rate", &self.0.sample_rate)
            .field("channels", &self.0.channels)
            .field("bits_per_sample", &self.0.bits_per_sample)
            .field("total_samples", &self.0.total_samples)
            .field("md5sum", &format_args!("{}", self.0.md5sum.iter().map(|x|{format!("{:02x}", x)}).collect::<Vec<String>>().join("")))
            .finish()
    }
}

#[derive(Clone, Copy)]
struct WrappedPadding(FLAC__StreamMetadata_Padding);
impl Debug for WrappedPadding {
    fn fmt(&self, fmt: &mut Formatter) -> fmt::Result {
        fmt.debug_struct("FLAC__StreamMetadata_Padding")
            .field("dummy", &self.0.dummy)
            .finish()
    }
}

#[derive(Clone, Copy)]
struct WrappedApplication(FLAC__StreamMetadata_Application, u32);
impl WrappedApplication {
    pub fn get_header(&self) -> String {
        String::from_utf8_lossy(&self.0.id).to_string()
    }
    pub fn get_data(&self) -> Vec<u8> {
        let n = self.1 - 4;
        unsafe {slice::from_raw_parts(self.0.data, n as usize)}.to_vec()
    }
}

impl Debug for WrappedApplication {
    fn fmt(&self, fmt: &mut Formatter) -> fmt::Result {
        fmt.debug_struct("FLAC__StreamMetadata_Application")
            .field("id", &self.get_header())
            .field("data", &String::from_utf8_lossy(&self.get_data()))
            .finish()
    }
}

#[derive(Clone, Copy)]
struct WrappedSeekPoint(FLAC__StreamMetadata_SeekPoint);
impl Debug for WrappedSeekPoint {
    fn fmt(&self, fmt: &mut Formatter) -> fmt::Result {
        fmt.debug_struct("FLAC__StreamMetadata_SeekPoint")
            .field("sample_number", &self.0.sample_number)
            .field("stream_offset", &self.0.stream_offset)
            .field("frame_samples", &self.0.frame_samples)
            .finish()
    }
}

#[derive(Clone, Copy)]
struct WrappedSeekTable(FLAC__StreamMetadata_SeekTable);
impl Debug for WrappedSeekTable {
    fn fmt(&self, fmt: &mut Formatter) -> fmt::Result {
        let points: Vec<WrappedSeekPoint> = unsafe {slice::from_raw_parts(self.0.points, self.0.num_points as usize).iter().map(|p|{WrappedSeekPoint(*p)}).collect()};
        fmt.debug_struct("FLAC__StreamMetadata_SeekTable")
            .field("num_points", &self.0.num_points)
            .field("points", &format_args!("{:?}", points))
            .finish()
    }
}

#[derive(Clone, Copy)]
struct WrappedVorbisComment(FLAC__StreamMetadata_VorbisComment);
impl Debug for WrappedVorbisComment {
    fn fmt(&self, fmt: &mut Formatter) -> fmt::Result {
        fmt.debug_struct("FLAC__StreamMetadata_VorbisComment")
            .field("vendor_string", &entry_to_string(&self.0.vendor_string))
            .field("num_comments", &self.0.num_comments)
            .field("comments", &format_args!("[{}]", (0..self.0.num_comments).map(|i|unsafe{entry_to_string(&*self.0.comments.add(i as usize))}).collect::<Vec<String>>().join(", ")))
            .finish()
    }
}

#[derive(Clone, Copy)]
struct WrappedCueSheet(FLAC__StreamMetadata_CueSheet);
impl Debug for WrappedCueSheet {
    fn fmt(&self, fmt: &mut Formatter) -> fmt::Result {
        fmt.debug_struct("FLAC__StreamMetadata_CueSheet")
            .field("media_catalog_number", &String::from_utf8_lossy(&self.0.media_catalog_number.into_iter().map(|c|{c as u8}).collect::<Vec<u8>>()))
            .field("lead_in", &self.0.lead_in)
            .field("is_cd", &self.0.is_cd)
            .field("num_tracks", &self.0.num_tracks)
            .field("tracks", &format_args!("[{}]", (0..self.0.num_tracks).map(|i|format!("{:?}", unsafe{*self.0.tracks.add(i as usize)})).collect::<Vec<String>>().join(", ")))
            .finish()
    }
}

#[derive(Clone, Copy)]
struct WrappedCueSheetTrack(FLAC__StreamMetadata_CueSheet_Track);
impl Debug for WrappedCueSheetTrack {
    fn fmt(&self, fmt: &mut Formatter) -> fmt::Result {
        fmt.debug_struct("FLAC__StreamMetadata_CueSheet_Track")
            .field("offset", &self.0.offset)
            .field("number", &self.0.number)
            .field("isrc", &self.0.isrc)
            .field("type", &self.0.type_())
            .field("pre_emphasis", &self.0.pre_emphasis())
            .field("num_indices", &self.0.num_indices)
            .field("indices", &format_args!("[{}]", (0..self.0.num_indices).map(|i|format!("{:?}", unsafe{*self.0.indices.add(i as usize)})).collect::<Vec<String>>().join(", ")))
            .finish()
    }
}

#[derive(Clone, Copy)]
struct WrappedCueSheetIndex(FLAC__StreamMetadata_CueSheet_Index);
impl Debug for WrappedCueSheetIndex {
    fn fmt(&self, fmt: &mut Formatter) -> fmt::Result {
        fmt.debug_struct("FLAC__StreamMetadata_CueSheet_Index")
            .field("offset", &self.0.offset)
            .field("number", &self.0.number)
            .finish()
    }
}

fn picture_type_to_str(pictype: u32) -> &'static str {
    match pictype {
        FLAC__STREAM_METADATA_PICTURE_TYPE_FILE_ICON_STANDARD => "32x32 pixels 'file icon' (PNG only)",
        FLAC__STREAM_METADATA_PICTURE_TYPE_FILE_ICON => "Other file icon",
        FLAC__STREAM_METADATA_PICTURE_TYPE_FRONT_COVER => "Cover (front)",
        FLAC__STREAM_METADATA_PICTURE_TYPE_BACK_COVER => "Cover (back)",
        FLAC__STREAM_METADATA_PICTURE_TYPE_LEAFLET_PAGE => "Leaflet page",
        FLAC__STREAM_METADATA_PICTURE_TYPE_MEDIA => "Media (e.g. label side of CD)",
        FLAC__STREAM_METADATA_PICTURE_TYPE_LEAD_ARTIST => "Lead artist/lead performer/soloist",
        FLAC__STREAM_METADATA_PICTURE_TYPE_ARTIST => "Artist/performer",
        FLAC__STREAM_METADATA_PICTURE_TYPE_CONDUCTOR => "Conductor",
        FLAC__STREAM_METADATA_PICTURE_TYPE_BAND => "Band/Orchestra",
        FLAC__STREAM_METADATA_PICTURE_TYPE_COMPOSER => "Composer",
        FLAC__STREAM_METADATA_PICTURE_TYPE_LYRICIST => "Lyricist/text writer",
        FLAC__STREAM_METADATA_PICTURE_TYPE_RECORDING_LOCATION => "Recording Location",
        FLAC__STREAM_METADATA_PICTURE_TYPE_DURING_RECORDING => "During recording",
        FLAC__STREAM_METADATA_PICTURE_TYPE_DURING_PERFORMANCE => "During performance",
        FLAC__STREAM_METADATA_PICTURE_TYPE_VIDEO_SCREEN_CAPTURE => "Movie/video screen capture",
        FLAC__STREAM_METADATA_PICTURE_TYPE_FISH => "A bright coloured fish",
        FLAC__STREAM_METADATA_PICTURE_TYPE_ILLUSTRATION => "Illustration",
        FLAC__STREAM_METADATA_PICTURE_TYPE_BAND_LOGOTYPE => "Band/artist logotype",
        FLAC__STREAM_METADATA_PICTURE_TYPE_PUBLISHER_LOGOTYPE => "Publisher/Studio logotype",
        _ => "Other",
    }
}

#[derive(Clone, Copy)]
struct WrappedPicture(FLAC__StreamMetadata_Picture);
impl Debug for WrappedPicture {
    fn fmt(&self, fmt: &mut Formatter) -> fmt::Result {
        fmt.debug_struct("FLAC__StreamMetadata_Picture")
            .field("type_", &picture_type_to_str(self.0.type_))
            .field("mime_type", &unsafe{CStr::from_ptr(self.0.mime_type).to_str()})
            .field("description", &unsafe{CStr::from_ptr(self.0.description as *const i8).to_str()})
            .field("width", &self.0.width)
            .field("height", &self.0.height)
            .field("depth", &self.0.depth)
            .field("colors", &self.0.colors)
            .field("data_length", &self.0.data_length)
            .field("data", &format_args!("[u8; {}]", self.0.data_length))
            .finish()
    }
}

#[derive(Clone, Copy)]
struct WrappedUnknown(FLAC__StreamMetadata_Unknown);
impl Debug for WrappedUnknown {
    fn fmt(&self, fmt: &mut Formatter) -> fmt::Result {
        fmt.debug_struct("FLAC__StreamMetadata_Unknown")
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy)]
struct WrappedStreamMetadata(FLAC__StreamMetadata);

impl Debug for WrappedStreamMetadata {
    fn fmt(&self, fmt: &mut Formatter) -> fmt::Result {
        fmt.debug_struct("FLAC__StreamMetadata")
            .field("type_", &self.0.type_)
            .field("is_last", &self.0.is_last)
            .field("length", &self.0.length)
            .field("data", &match self.0.type_ {
                FLAC__METADATA_TYPE_STREAMINFO => format!("{:?}", unsafe{WrappedStreamInfo(self.0.data.stream_info)}),
                FLAC__METADATA_TYPE_PADDING => format!("{:?}", unsafe{WrappedPadding(self.0.data.padding)}),
                FLAC__METADATA_TYPE_APPLICATION => format!("{:?}", unsafe{WrappedApplication(self.0.data.application, self.0.length)}),
                FLAC__METADATA_TYPE_SEEKTABLE => format!("{:?}", unsafe{WrappedSeekTable(self.0.data.seek_table)}),
                FLAC__METADATA_TYPE_VORBIS_COMMENT => format!("{:?}", unsafe{WrappedVorbisComment(self.0.data.vorbis_comment)}),
                FLAC__METADATA_TYPE_CUESHEET => format!("{:?}", unsafe{WrappedCueSheet(self.0.data.cue_sheet)}),
                FLAC__METADATA_TYPE_PICTURE => format!("{:?}", unsafe{WrappedPicture(self.0.data.picture)}),
                FLAC__METADATA_TYPE_UNDEFINED => format!("{:?}", unsafe{WrappedUnknown(self.0.data.unknown)}),
                o => format!("Unknown metadata type {o}"),
            })
            .finish()
    }
}
