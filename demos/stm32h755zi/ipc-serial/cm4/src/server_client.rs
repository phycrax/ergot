#![allow(async_fn_in_trait)]

use core::pin::pin;

use comms::{
    GreenLedEndpoint, PairedUartProfile, RedLedEndpoint, RxIdle, RxWorker, TxIdle, TxWorker
};
use embassy_stm32::{
    gpio::Output,
    mode::{self, Async},
    usart::{self, UartRx, UartTx},
};
use embassy_time::{Duration, Ticker};
use ergot::{exports::bbq2::{queue::BBQueue, traits::{notifier::maitake::MaiNotSpsc, storage::Inline}}, Address, NetStack};
use ergot::exports::{bbq2::traits::coordination::cas::AtomicCoord,mutex::raw_impls::{cs::CriticalSectionRawMutex}};
use static_cell::ConstStaticCell;

pub const TX_QUEUE_LEN: usize = 4096;
pub const RX_BUF_LEN: usize = 512;

pub type TxQueue = BBQueue<Inline<TX_QUEUE_LEN>, AtomicCoord, MaiNotSpsc>;
pub type Stack = NetStack<CriticalSectionRawMutex, PairedUartProfile<&'static TxQueue>>;

pub static TX_QUEUE: TxQueue = TxQueue::new();
pub static STACK: Stack = PairedUartProfile::new_target_stack::<CriticalSectionRawMutex>(
    TX_QUEUE.framed_producer(),
    RX_BUF_LEN as u16,
);

#[embassy_executor::task]
pub async fn tx_task(tx: UartTx<'static, Async>) {
    let mut tx_worker = TxWorker::new_target(&STACK, &TX_QUEUE, WrappedTx(tx))
        .map_err(drop)
        .unwrap();

    loop {
        tx_worker.run_until_err().await;
    }
}

#[embassy_executor::task]
pub async fn rx_task(rx: UartRx<'static, Async>) {
    static RX_BUF: ConstStaticCell<[u8; RX_BUF_LEN]> = ConstStaticCell::new([0u8; RX_BUF_LEN]);

    let mut rx_worker = RxWorker::new_target(&STACK, WrappedRx(rx), RX_BUF.take());

    rx_worker.run().await;
}

#[embassy_executor::task]
pub async fn led_server(mut led: Output<'static>) {
    let socket = STACK.stack_bounded_endpoint_server::<RedLedEndpoint, 2>(Some("redled"));
    let socket = pin!(socket);
    let mut hdl = socket.attach();

    loop {
        let _ = hdl
            .serve(async |on| {
                if *on {
                    led.set_low();
                } else {
                    led.set_high();
                }
            })
            .await;
    }
}

#[embassy_executor::task]
pub async fn led_client() {
    let mut ticker = Ticker::every(Duration::from_millis(500));
    let dst = Address {
        network_id: 1,
        node_id: 1,
        port_id: 0,
    };

    loop {
        ticker.next().await;
        let _ = STACK
            .req_resp::<GreenLedEndpoint>(dst, &true, Some("greenled"))
            .await;
        ticker.next().await;
        let _ = STACK
            .req_resp::<GreenLedEndpoint>(dst, &false, Some("greenled"))
            .await;
    }
}

struct WrappedTx<'a>(UartTx<'a, mode::Async>);

impl TxIdle for WrappedTx<'_> {
    type Error = usart::Error;

    async fn send_all(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        self.0.write(data).await?;
        Ok(())
    }
}

struct WrappedRx<'a>(UartRx<'a, mode::Async>);

impl RxIdle for WrappedRx<'_> {
    type Error = usart::Error;

    async fn recv_until_idle<'a>(
        &mut self,
        buf: &'a mut [u8],
    ) -> Result<&'a mut [u8], Self::Error> {
        let got = self.0.read_until_idle(buf).await?;
        Ok(&mut buf[..got])
    }
}
