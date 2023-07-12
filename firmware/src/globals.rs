use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
use serial_ws2812_shared::{BYTES_PER_LED, MAX_LEDS_PER_STRIP, MAX_STRIPS};

pub type LEDs = [[[u8; BYTES_PER_LED]; MAX_LEDS_PER_STRIP]; MAX_STRIPS];

pub type DisplayCommand = (usize, &'static mut LEDs);

pub static DISPLAY_CHANNEL: Channel<CriticalSectionRawMutex, DisplayCommand, 1> = Channel::new();
pub static RETURN_CHANNEL: Channel<CriticalSectionRawMutex, &'static mut LEDs, 1> = Channel::new();
