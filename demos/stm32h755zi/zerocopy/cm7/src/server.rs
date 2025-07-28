#![allow(async_fn_in_trait)]

use core::pin::pin;

use comms::{
    self, Channel, GreenLedEndpoint, PacketBuf, PairedUartProfile, Receiver, RedLedEndpoint,
    RxIdle, RxWorker, TxIdle, TxWorker,
};

use embassy_stm32::{
    gpio::Output,
    mode::{self, Async},
    usart::{self, Uart, UartRx, UartTx},
};
use embassy_sync::{
    blocking_mutex::raw::{self, NoopRawMutex},
    zerocopy_channel,
};
use embassy_time::{Duration, Ticker};
use ergot::{Address, NetStack, exports::mutex::raw_impls::local::LocalRawMutex};
use static_cell::{ConstStaticCell, StaticCell};

pub const TX_QUEUE_LEN: usize = 16;
pub const RX_BUF_LEN: usize = 512;
pub const MTU: usize = 252;
pub type Stack<'ch> = NetStack<LocalRawMutex, PairedUartProfile<'ch, NoopRawMutex, MTU>>;

pub fn init(spawner: embassy_executor::Spawner, uart: Uart<'static, Async>, led: Output<'static>) {
    static STACK: StaticCell<Stack<'static>> = StaticCell::new();
    static TX_BUF: ConstStaticCell<[PacketBuf<MTU>; TX_QUEUE_LEN]> =
        ConstStaticCell::new([const { PacketBuf::new() }; TX_QUEUE_LEN]);

    static CHANNEL: StaticCell<Channel<raw::NoopRawMutex, MTU>> = StaticCell::new();

    let channel = zerocopy_channel::Channel::new(TX_BUF.take());

    let (ch_sender, ch_receiver) = CHANNEL.init(channel).split();

    let stack = STACK.init(PairedUartProfile::new_controller_stack(ch_sender));

    let (tx, rx) = uart.split();
    {
        spawner.must_spawn(tx_task(stack, ch_receiver, tx));
        spawner.must_spawn(rx_task(stack, rx));
        spawner.must_spawn(led_server(stack, led));
        spawner.must_spawn(led_client(stack));
    }
}

#[embassy_executor::task]
pub async fn tx_task(
    stack: &'static Stack<'static>,
    ch_receiver: Receiver<'static, raw::NoopRawMutex, MTU>,
    tx: UartTx<'static, Async>,
) {
    let mut tx_worker = TxWorker::new_controller(stack, ch_receiver, WrappedTx(tx));

    loop {
        tx_worker.run_until_err().await;
    }
}

#[embassy_executor::task]
pub async fn rx_task(stack: &'static Stack<'static>, rx: UartRx<'static, Async>) {
    static RX_BUF: ConstStaticCell<[u8; RX_BUF_LEN]> = ConstStaticCell::new([0u8; RX_BUF_LEN]);

    let mut rx_worker = RxWorker::new_controller(stack, WrappedRx(rx), RX_BUF.take());

    rx_worker.run().await;
}

#[embassy_executor::task]
pub async fn led_server(stack: &'static Stack<'static>, mut led: Output<'static>) {
    let socket = stack.stack_bounded_endpoint_server::<GreenLedEndpoint, 2>(Some("greenled"));
    let socket = pin!(socket);
    let mut hdl = socket.attach();

    loop {
        let _ = hdl
            .serve(async |on| {
                defmt::info!("LED set {=bool}", *on);
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
pub async fn led_client(stack: &'static Stack<'static>) {
    let mut ticker = Ticker::every(Duration::from_millis(500));
    let dst = Address {
        network_id: 1,
        node_id: 2,
        port_id: 0,
    };

    loop {
        ticker.next().await;
        let _ = stack
            .req_resp::<RedLedEndpoint>(dst, &true, Some("redled"))
            .await;
        ticker.next().await;
        let _ = stack
            .req_resp::<RedLedEndpoint>(dst, &false, Some("redled"))
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
