#![no_std]
#![allow(async_fn_in_trait)]

use core::marker::PhantomData;

use defmt::{assert, debug, warn};
use embassy_sync::{blocking_mutex::raw::RawMutex, zerocopy_channel};
use ergot::{
    NetStack, endpoint,
    ergot_base::{
        FrameKind, Header, NetStackSendError, ProtocolError,
        net_stack::NetStackHandle,
        wire_frames::{self, CommonHeader, de_frame},
    },
    exports::mutex::raw_impls::local::LocalRawMutex,
    interface_manager::{
        Interface, InterfaceSendError, InterfaceSink, InterfaceState, Profile,
        profiles::direct_edge::{CENTRAL_NODE_ID, DirectEdge, EDGE_NODE_ID},
    },
};
use postcard::ser_flavors::Slice;
use serde::Serialize;

endpoint!(RedLedEndpoint, bool, (), "redled/set");
endpoint!(GreenLedEndpoint, bool, (), "greenled/set");

pub type Channel<'ch, M, const MTU: usize> = zerocopy_channel::Channel<'ch, M, PacketBuf<MTU>>;
pub type Receiver<'ch, M, const MTU: usize> = zerocopy_channel::Receiver<'ch, M, PacketBuf<MTU>>;
pub type Sender<'ch, M, const MTU: usize> = zerocopy_channel::Sender<'ch, M, PacketBuf<MTU>>;

/// Represents a packet of size MTU.
pub struct PacketBuf<const MTU: usize> {
    len: usize,
    buf: [u8; MTU],
}

impl<const MTU: usize> PacketBuf<MTU> {
    /// Create a new packet buffer.
    pub const fn new() -> Self {
        Self {
            len: 0,
            buf: [0; MTU],
        }
    }
}

pub struct IdleUartSink<'ch, M: RawMutex, const MTU: usize> {
    pub ch_sender: Sender<'ch, M, MTU>,
}

impl<'ch, M: RawMutex, const MTU: usize> InterfaceSink for IdleUartSink<'ch, M, MTU> {
    fn send_ty<T: serde::Serialize>(
        &mut self,
        hdr: &CommonHeader,
        apdx: Option<&ergot::ergot_base::AnyAllAppendix>,
        body: &T,
    ) -> Result<(), ()> {
        let is_err = hdr.kind == FrameKind::PROTOCOL_ERROR;

        if is_err {
            // todo: use a different interface for this
            return Err(());
        }
        let packet = self.ch_sender.try_send().ok_or(())?;

        let ser = Slice::new(&mut packet.buf);
        let used = wire_frames::encode_frame_ty(ser, &hdr, apdx, body).map_err(drop)?;
        packet.len = used.len();
        self.ch_sender.send_done();

        Ok(())
    }

    fn send_raw(&mut self, hdr: &CommonHeader, hdr_raw: &[u8], body: &[u8]) -> Result<(), ()> {
        let is_err = hdr.kind == FrameKind::PROTOCOL_ERROR;

        if is_err {
            // todo: use a different interface for this
            return Err(());
        }
        let len = hdr_raw.len() + body.len();
        let Ok(len) = u16::try_from(len) else {
            return Err(());
        };
        let packet = self.ch_sender.try_send().ok_or(())?;
        let (ghdr, gbody) = packet.buf.split_at_mut(hdr_raw.len());
        ghdr.copy_from_slice(hdr_raw);
        gbody.copy_from_slice(body);

        packet.len = len as usize;
        self.ch_sender.send_done();

        Ok(())
    }

    fn send_err(&mut self, hdr: &CommonHeader, err: ProtocolError) -> Result<(), ()> {
        let is_err = hdr.kind == FrameKind::PROTOCOL_ERROR;

        // note: here it SHOULD be an err!
        if !is_err {
            // todo: use a different interface for this
            return Err(());
        }
        let packet = self.ch_sender.try_send().ok_or(())?;

        let ser = Slice::new(&mut packet.buf);
        let used = wire_frames::encode_frame_err(ser, hdr, err).map_err(drop)?;
        packet.len = used.len();
        self.ch_sender.send_done();

        Ok(())
    }
}

pub struct IdleUartInterface<'ch, M: RawMutex, const MTU: usize> {
    _pd: PhantomData<[&'ch M; MTU]>,
}

impl<'ch, M: RawMutex, const MTU: usize> Interface for IdleUartInterface<'ch, M, MTU> {
    type Sink = IdleUartSink<'ch, M, MTU>;
}

pub struct PairedUartProfile<'ch, M: RawMutex, const MTU: usize> {
    inner: DirectEdge<IdleUartInterface<'ch, M, MTU>>,
}

impl<'ch, M: RawMutex, const MTU: usize> PairedUartProfile<'ch, M, MTU> {
    pub const fn new_controller_stack(
        ch_sender: Sender<'ch, M, MTU>,
    ) -> NetStack<LocalRawMutex, Self> {
        NetStack::const_new(
            LocalRawMutex::new(),
            Self {
                inner: DirectEdge::new_controller(IdleUartSink { ch_sender }, InterfaceState::Down),
            },
        )
    }

    pub const fn new_target_stack(ch_sender: Sender<'ch, M, MTU>) -> NetStack<LocalRawMutex, Self> {
        NetStack::const_new(
            LocalRawMutex::new(),
            Self {
                inner: DirectEdge::new_target(IdleUartSink { ch_sender }),
            },
        )
    }
}

impl<'ch, M: RawMutex, const MTU: usize> Profile for PairedUartProfile<'ch, M, MTU> {
    type InterfaceIdent = ();

    fn send<T: Serialize>(&mut self, hdr: &Header, data: &T) -> Result<(), InterfaceSendError> {
        self.inner.send(hdr, data)
    }

    fn send_err(&mut self, hdr: &Header, err: ProtocolError) -> Result<(), InterfaceSendError> {
        self.inner.send_err(hdr, err)
    }

    fn send_raw(
        &mut self,
        hdr: &Header,
        hdr_raw: &[u8],
        data: &[u8],
    ) -> Result<(), InterfaceSendError> {
        self.inner.send_raw(hdr, hdr_raw, data)
    }

    fn interface_state(&mut self, ident: Self::InterfaceIdent) -> Option<InterfaceState> {
        self.inner.interface_state(ident)
    }

    fn set_interface_state(
        &mut self,
        ident: Self::InterfaceIdent,
        state: InterfaceState,
    ) -> Result<(), ergot::interface_manager::SetStateError> {
        self.inner.set_interface_state(ident, state)
    }
}

pub trait TxIdle {
    type Error;
    async fn send_all(&mut self, data: &[u8]) -> Result<(), Self::Error>;
}

pub trait RxIdle {
    type Error;
    async fn recv_until_idle<'a>(&mut self, buf: &'a mut [u8])
    -> Result<&'a mut [u8], Self::Error>;
}

pub struct TxWorker<'ch, M, TX, const MTU: usize>
where
    M: RawMutex,
    TX: TxIdle,
{
    ch_receiver: Receiver<'ch, M, MTU>,
    uart_tx: TX,
}

impl<'ch, M, TX, const MTU: usize> TxWorker<'ch, M, TX, MTU>
where
    M: RawMutex,
    TX: TxIdle,
{
    pub fn new_controller<N>(net: N, ch_receiver: Receiver<'ch, M, MTU>, uart_tx: TX) -> Self
    where
        N: NetStackHandle<Profile = PairedUartProfile<'ch, M, MTU>>,
    {
        let res = net.stack().manage_profile(|mgr| {
            mgr.set_interface_state(
                (),
                InterfaceState::Active {
                    net_id: 1,
                    node_id: CENTRAL_NODE_ID,
                },
            )
        });

        assert!(res.is_ok());

        Self {
            ch_receiver,
            uart_tx,
        }
    }

    pub fn new_target<N>(net: N, ch_receiver: Receiver<'ch, M, MTU>, uart_tx: TX) -> Self
    where
        N: NetStackHandle<Profile = PairedUartProfile<'ch, M, MTU>>,
    {
        let res = net
            .stack()
            .manage_profile(|mgr| mgr.set_interface_state((), InterfaceState::Inactive));

        assert!(res.is_ok());

        Self {
            ch_receiver,
            uart_tx,
        }
    }

    pub async fn run_until_err(&mut self) -> TX::Error {
        loop {
            let rx = self.ch_receiver.receive().await;
            let frame = &rx.buf[..rx.len];
            let res = self.uart_tx.send_all(frame).await;
            self.ch_receiver.receive_done();
            if let Err(e) = res {
                return e;
            }
        }
    }
}

pub struct RxWorker<'buf, 'ch, N, M, RX, const MTU: usize>
where
    N: NetStackHandle<Profile = PairedUartProfile<'ch, M, MTU>>,
    M: RawMutex + 'ch,
    RX: RxIdle,
{
    nsh: N,
    uart_rx: RX,
    rx_buf: &'buf mut [u8],
    is_controller: bool,
    net_id: Option<u16>,
}

impl<'buf, 'ch, N, M, RX, const MTU: usize> RxWorker<'buf, 'ch, N, M, RX, MTU>
where
    N: NetStackHandle<Profile = PairedUartProfile<'ch, M, MTU>>,
    M: RawMutex + 'ch,
    RX: RxIdle,
{
    pub fn new_controller(nsh: N, uart_rx: RX, rx_buf: &'buf mut [u8]) -> Self {
        Self {
            nsh,
            uart_rx,
            rx_buf,
            is_controller: true,
            net_id: Some(1),
        }
    }

    pub fn new_target(nsh: N, uart_rx: RX, rx_buf: &'buf mut [u8]) -> Self {
        Self {
            nsh,
            uart_rx,
            rx_buf,
            is_controller: false,
            net_id: None,
        }
    }

    #[inline]
    pub fn own_node_id(&self) -> u8 {
        if self.is_controller { 1 } else { 2 }
    }

    pub async fn run(&mut self) -> ! {
        let own_node_id = self.own_node_id();

        let Self {
            nsh,
            uart_rx,
            rx_buf,
            is_controller,
            net_id,
        } = self;
        loop {
            let Ok(f) = uart_rx.recv_until_idle(rx_buf).await else {
                warn!("recv error");
                continue;
            };

            let Some(mut frame) = de_frame(f) else {
                warn!(
                    "Decode error! Ignoring frame on net_id {}",
                    net_id.unwrap_or(0)
                );
                continue;
            };

            debug!("Got Frame!");

            let take_net = !*is_controller
                && (net_id.is_none()
                    || net_id.is_some_and(|n| {
                        frame.hdr.dst.network_id != 0 && n != frame.hdr.dst.network_id
                    }));

            if take_net {
                nsh.stack().manage_profile(|im| {
                    _ = im.set_interface_state(
                        (),
                        InterfaceState::Active {
                            net_id: frame.hdr.dst.network_id,
                            node_id: if *is_controller {
                                CENTRAL_NODE_ID
                            } else {
                                EDGE_NODE_ID
                            },
                        },
                    );
                });
                *net_id = Some(frame.hdr.dst.network_id);
            }

            // If the message comes in and has a src net_id of zero,
            // we should rewrite it so it isn't later understood as a
            // local packet.
            //
            // TODO: accept any packet if we don't have a net_id yet?
            if let Some(net) = net_id.as_ref() {
                if frame.hdr.src.network_id == 0 {
                    assert_ne!(frame.hdr.src.node_id, 0, "we got a local packet remotely?");
                    assert_ne!(
                        frame.hdr.src.node_id, own_node_id,
                        "someone is pretending to be us?"
                    );

                    frame.hdr.src.network_id = *net;
                }
            }

            // TODO: if the destination IS self.net_id, we could rewrite the
            // dest net_id as zero to avoid a pass through the interface manager.
            //
            // If the dest is 0, should we rewrite the dest as self.net_id? This
            // is the opposite as above, but I dunno how that will work with responses
            let hdr = frame.hdr.clone();
            let hdr: Header = hdr.into();
            let res = match frame.body {
                Ok(body) => nsh.stack().send_raw(&hdr, frame.hdr_raw, body),
                Err(e) => nsh.stack().send_err(&hdr, e),
            };
            match res {
                Ok(()) => {}
                Err(e) => {
                    // TODO: match on error, potentially try to send NAK?
                    match e {
                        NetStackSendError::SocketSend(_) => {
                            warn!("SocketSend(SocketSendError");
                        }
                        NetStackSendError::InterfaceSend(_) => {
                            warn!("InterfaceSend(InterfaceSendError");
                        }
                        NetStackSendError::NoRoute => {
                            warn!("NoRoute");
                        }
                        NetStackSendError::AnyPortMissingKey => {
                            warn!("AnyPortMissingKey");
                        }
                        NetStackSendError::WrongPortKind => {
                            warn!("WrongPortKind");
                        }
                        NetStackSendError::AnyPortNotUnique => {
                            warn!("AnyPortNotUnique");
                        }
                        NetStackSendError::AllPortMissingKey => {
                            warn!("AllPortMissingKey");
                        }
                        _ => warn!("OTHER"),
                    }
                }
            }
        }
    }
}
