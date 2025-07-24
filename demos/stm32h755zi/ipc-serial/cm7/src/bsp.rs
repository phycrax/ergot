pub mod irqs {
    use embassy_stm32::{bind_interrupts, peripherals, usart};

    bind_interrupts!(pub struct Serial1 {
        USART1 => usart::InterruptHandler<peripherals::USART1>;
    });
}

pub mod peripherals {
    use embassy_stm32::{Peri, peripherals};

    assign_resources::assign_resources! {
        led: Led {
            green: PB0,
        }
        serial_1: Serial1 {
            usart: USART1,
            rx: PB7,
            tx: PB6,
            tx_dma: DMA1_CH0,
            rx_dma: DMA1_CH1,
        }
    }
}
