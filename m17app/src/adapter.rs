use crate::{app::TxHandle, link_setup::LinkSetup};
use m17core::protocol::PacketType;
use std::sync::Arc;

/// Can be connected to an `M17App` to receive incoming packet data.
///
/// The `packet_received` callback will be fired once for each incoming packet type. Any filtering
/// must be done by the receiver. There are also some lifecycle callbacks, one of which will provide
/// a `TxHandle` when the adapter is first added to the app. This means the adapter can transmit as
/// well as receive.
pub trait PacketAdapter: Send + Sync + 'static {
    /// This adapter was added to an `M17App`.
    fn adapter_registered(&self, id: usize, handle: TxHandle) {
        let _ = id;
        let _ = handle;
    }

    /// This adapter was removed from an `M17App`.
    fn adapter_removed(&self) {}

    /// The TNC has been started and incoming packets may now arrive.
    fn tnc_started(&self) {}

    /// The TNC has been shut down. There will be no more tx/rx.
    fn tnc_closed(&self) {}

    /// A packet has been received and assembled by the radio.
    fn packet_received(&self, link_setup: LinkSetup, packet_type: PacketType, content: Arc<[u8]>) {
        let _ = link_setup;
        let _ = packet_type;
        let _ = content;
    }
}

/// Can be connected to an `M17App` to receive incoming streams (voice or data).
///
/// Once an incoming stream has been acquired (either by receiving a Link Setup Frame or by decoding
/// an ongoing LICH), all stream frames will be provided to this adapter.
///
/// There are also some lifecycle callbacks, one of which will provide a `TxHandle` when the adapter
/// is first added to the app. This means the adapter can transmit as well as receive.
pub trait StreamAdapter: Send + Sync + 'static {
    /// This adapter was added to an `M17App`.
    fn adapter_registered(&self, id: usize, handle: TxHandle) {
        let _ = id;
        let _ = handle;
    }

    /// This adapter was removed from an `M17App`.
    fn adapter_removed(&self) {}

    /// The TNC has been started and incoming streams may now arrive.
    fn tnc_started(&self) {}

    /// The TNC has been shut down. There will be no more tx/rx.
    fn tnc_closed(&self) {}

    /// A new incoming stream has begun.
    ///
    /// If we did not receive the end of the previous stream, this may occur even there was never a
    /// `stream_data` where `is_final` is true.
    fn stream_began(&self, link_setup: LinkSetup) {
        let _ = link_setup;
    }

    /// A frame has been received for an ongoing incoming stream.
    ///
    /// It is not guaranteed to receive every frame. Frame numbers may not start from 0, and they will
    /// wrap around to 0 after 0x7fff. If we receive an indication that the frame is the final one then
    /// `is_final` is set. If the transmitter never sends that frame or we fail to receive it then the
    /// stream may trail off without that being set. Implementors should consider setting an appropriate
    /// timeout to consider a stream "dead" and wait for the next `stream_began`.
    fn stream_data(&self, frame_number: u16, is_final: bool, data: Arc<[u8; 16]>) {
        let _ = frame_number;
        let _ = is_final;
        let _ = data;
    }

    // TODO
    // fn stream_lost(&self);
    // fn stream_assembled_text_block()
    // fn stream_gnss_data()
    // fn stream_extended_callsign_data()

    // fn stream_tx_ended_early(&self); // underrun/overrun
}
