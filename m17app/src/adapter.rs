use crate::app::TxHandle;
use m17core::protocol::{LsfFrame, PacketType};
use std::sync::Arc;

pub trait PacketAdapter: Send + Sync + 'static {
    fn adapter_registered(&self, handle: TxHandle);
    fn tnc_started(&self);
    fn tnc_closed(&self);
    fn packet_received(&self, lsf: LsfFrame, packet_type: PacketType, content: Arc<[u8]>);
}

pub trait StreamAdapter: Send + Sync + 'static {}
