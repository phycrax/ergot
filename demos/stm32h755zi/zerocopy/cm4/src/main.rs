#![no_std]
#![no_main]

mod bsp;
mod server;

use core::mem::MaybeUninit;

use defmt::*;
use embassy_stm32::{
    gpio::{Level, Output, Speed},
    usart::Uart,
};

use {defmt_rtt as _, panic_probe as _};

#[unsafe(link_section = ".ram_d3.shared_data")]
static SHARED_DATA: MaybeUninit<embassy_stm32::SharedData> = MaybeUninit::uninit();

#[embassy_executor::main]
async fn main(spawner: embassy_executor::Spawner) {
    // Split peripherals
    let r = {
        let p = embassy_stm32::init_secondary(&SHARED_DATA);
        use bsp::peripherals::*;
        split_resources!(p)
    };

    // Hello world
    info!("Hello World! CM4");

    let uart = Uart::new(
        r.serial_7.uart,
        r.serial_7.rx,
        r.serial_7.tx,
        bsp::irqs::Serial7,
        r.serial_7.tx_dma,
        r.serial_7.rx_dma,
        Default::default(),
    )
    .unwrap();

    let led = Output::new(r.led.red, Level::Low, Speed::Low);

    server::init(spawner, uart, led);
}
