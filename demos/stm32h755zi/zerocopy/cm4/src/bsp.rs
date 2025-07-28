pub mod irqs {
    use embassy_stm32::{bind_interrupts, peripherals, usart};

    bind_interrupts!(pub struct Serial7 {
        UART7 => usart::InterruptHandler<peripherals::UART7>;
    });
}

pub mod peripherals {
    use embassy_stm32::{Peri, peripherals};

    assign_resources::assign_resources! {
        led: Led {
            red: PB14,
        }
        serial_7: Serial7 {
            uart: UART7,
            rx: PE7,
            tx: PE8,
            tx_dma: DMA2_CH0,
            rx_dma: DMA2_CH1,
        }
    }
}
