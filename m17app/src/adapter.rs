use crate::{app::TxHandle, link_setup::LinkSetup};
use m17core::protocol::PacketType;
use std::sync::Arc;

pub trait PacketAdapter: Send + Sync + 'static {
    fn adapter_registered(&self, id: usize, handle: TxHandle) {
        let _ = id;
        let _ = handle;
    }
    fn adapter_removed(&self) {}
    fn tnc_started(&self) {}
    fn tnc_closed(&self) {}
    fn packet_received(&self, link_setup: LinkSetup, packet_type: PacketType, content: Arc<[u8]>) {
        let _ = link_setup;
        let _ = packet_type;
        let _ = content;
    }
}

pub trait StreamAdapter: Send + Sync + 'static {
    fn adapter_registered(&self, id: usize, handle: TxHandle) {
        let _ = id;
        let _ = handle;
    }
    fn adapter_removed(&self) {}
    fn tnc_started(&self) {}
    fn tnc_closed(&self) {}
    fn stream_began(&self, link_setup: LinkSetup) {
        let _ = link_setup;
    }
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
