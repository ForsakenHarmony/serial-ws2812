#![no_std]

pub const MESSAGE_TYPE_LEN: usize = 8;
pub const MESSAGE_NUM_LEN: usize = 4;

pub const UPDATE_MESSAGE: &[u8; MESSAGE_TYPE_LEN] = b"update\0\0";
pub const SET_STRIPS_MESSAGE: &[u8; MESSAGE_TYPE_LEN] = b"strips\0\0";
pub const SET_LEDS_MESSAGE: &[u8; MESSAGE_TYPE_LEN] = b"leds\0\0\0\0";

/// This has to be 8 because the PIO "script" always writes 8 strips in parallel.
pub const MAX_STRIPS: usize = 8;
/// This could be increased, but you will get less than 60 updates per second.
pub const MAX_LEDS_PER_STRIP: usize = 512;
pub const BYTES_PER_LED: usize = 3;

pub const MAX_BUFFER_SIZE: usize = BYTES_PER_LED * MAX_LEDS_PER_STRIP * MAX_STRIPS;

pub const DEVICE_MESSAGE_TYPE_LEN: usize = 1;

pub const DEVICE_INIT_MESSAGE: &[u8; DEVICE_MESSAGE_TYPE_LEN] = b"i";
pub const DEVICE_ERROR_MESSAGE: &[u8; DEVICE_MESSAGE_TYPE_LEN] = b"e";
pub const DEVICE_PARTIAL_MESSAGE: &[u8; DEVICE_MESSAGE_TYPE_LEN] = b"p";
pub const DEVICE_OK_MESSAGE: &[u8; DEVICE_MESSAGE_TYPE_LEN] = b"k";

pub const DEVICE_PRODUCT_NAME: &str = "Serial WS2812";
