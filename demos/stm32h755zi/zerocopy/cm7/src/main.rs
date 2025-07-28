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
    // Setup RCC
    let mut config = embassy_stm32::Config::default();
    {
        use embassy_stm32::rcc::*;
        let rcc = &mut config.rcc;
        // HSI 400 MHz
        rcc.hsi = Some(HSIPrescaler::DIV1);
        rcc.csi = true;
        rcc.pll1 = Some(Pll {
            source: PllSource::HSI,
            prediv: PllPreDiv::DIV4,
            mul: PllMul::MUL50,
            divp: Some(PllDiv::DIV2),
            divq: None,
            divr: None,
        });
        rcc.sys = Sysclk::PLL1_P;
        rcc.ahb_pre = AHBPrescaler::DIV2;
        rcc.apb1_pre = APBPrescaler::DIV2;
        rcc.apb2_pre = APBPrescaler::DIV2;
        rcc.apb3_pre = APBPrescaler::DIV2;
        rcc.apb4_pre = APBPrescaler::DIV2;
        rcc.voltage_scale = VoltageScale::Scale1;
        rcc.supply_config = SupplyConfig::DirectSMPS;
    }

    // Split peripherals
    let r = {
        let p = embassy_stm32::init_primary(config, &SHARED_DATA);
        use bsp::peripherals::*;
        split_resources!(p)
    };

    // Hello world
    info!("Hello World! CM7 uid:{}", embassy_stm32::uid::uid());

    let uart = Uart::new(
        r.serial_1.usart,
        r.serial_1.rx,
        r.serial_1.tx,
        bsp::irqs::Serial1,
        r.serial_1.tx_dma,
        r.serial_1.rx_dma,
        Default::default(),
    )
    .unwrap();

    let led = Output::new(r.led.green, Level::Low, Speed::Low);

    server::init(spawner, uart, led);
}
