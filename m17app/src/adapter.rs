use crate::{app::TxHandle, link_setup::LinkSetup};
use m17core::protocol::PacketType;
use std::sync::Arc;

pub trait PacketAdapter: Send + Sync + 'static {
    fn adapter_registered(&self, _id: usize, _handle: TxHandle) {}
    fn adapter_removed(&self) {}
    fn tnc_started(&self) {}
    fn tnc_closed(&self) {}
    fn packet_received(
        &self,
        _link_setup: LinkSetup,
        _packet_type: PacketType,
        _content: Arc<[u8]>,
    ) {
    }
}

pub trait StreamAdapter: Send + Sync + 'static {
    fn adapter_registered(&self, _id: usize, _handle: TxHandle) {}
    fn adapter_removed(&self) {}
    fn tnc_started(&self) {}
    fn tnc_closed(&self) {}
    fn stream_began(&self, _link_setup: LinkSetup) {}
    fn stream_data(&self, _frame_number: u16, _is_final: bool, _data: Arc<[u8; 16]>) {}

    // TODO
    // fn stream_lost(&self);
    // fn stream_assembled_text_block()
    // fn stream_gnss_data()
    // fn stream_extended_callsign_data()

    // fn stream_tx_ended_early(&self); // underrun/overrun
}
