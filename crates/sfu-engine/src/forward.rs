use crate::ids::ClientId;
use rtc::rtp_transceiver::RTCRtpSenderId;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ForwardKey {
    pub publisher: ClientId,
    pub track: u64,
}

#[derive(Debug, Default)]
pub struct ForwardEntry {
    pub subscribers: HashMap<ClientId, RTCRtpSenderId>,
}

#[derive(Debug, Default)]
pub struct ForwardTable {
    entries: HashMap<ForwardKey, ForwardEntry>,
}

impl ForwardTable {
    pub fn entry_mut(&mut self, key: ForwardKey) -> &mut ForwardEntry {
        self.entries.entry(key).or_default()
    }

    pub fn get(&self, key: &ForwardKey) -> Option<&ForwardEntry> {
        self.entries.get(key)
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
