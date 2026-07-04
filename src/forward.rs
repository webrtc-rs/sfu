use rtc::rtp_transceiver::RTCRtpSenderId;
use std::collections::HashMap;

use crate::client::ClientId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ForwardKey {
    publisher: ClientId,
    track: u64,
}

#[derive(Debug, Default)]
pub(crate) struct ForwardEntry {
    subscribers: HashMap<ClientId, RTCRtpSenderId>,
}

#[derive(Debug, Default)]
pub(crate) struct ForwardTable {
    entries: HashMap<ForwardKey, ForwardEntry>,
}

impl ForwardTable {
    pub(crate) fn entry_mut(&mut self, key: ForwardKey) -> &mut ForwardEntry {
        self.entries.entry(key).or_default()
    }

    pub(crate) fn get(&self, key: &ForwardKey) -> Option<&ForwardEntry> {
        self.entries.get(key)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
