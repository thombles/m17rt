use crate::app::TxHandle;
use m17core::protocol::{LsfFrame, PacketType};
use std::sync::Arc;

pub trait PacketAdapter: Send + Sync + 'static {
    fn adapter_registered(&self, id: usize, handle: TxHandle);
    fn adapter_removed(&self);
    fn tnc_started(&self);
    fn tnc_closed(&self);
    fn packet_received(&self, lsf: LsfFrame, packet_type: PacketType, content: Arc<[u8]>);
}

pub trait StreamAdapter: Send + Sync + 'static {
    fn adapter_registered(&self, id: usize, handle: TxHandle);
    fn adapter_removed(&self);
    fn tnc_started(&self);
    fn tnc_closed(&self);
    fn stream_began(&self, lsf: LsfFrame);
    fn stream_data(&self, frame_number: u16, is_final: bool, data: Arc<[u8; 16]>);

    // TODO
    // fn stream_lost(&self);
    // fn stream_assembled_text_block()
    // fn stream_gnss_data()
    // fn stream_extended_callsign_data()
}
