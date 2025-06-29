use bop_common::{telemetry::system::SystemNotification, time::Nanos};
use ratatui::text::Text;
use serde::{Deserialize, Serialize};

use super::Data;
use crate::{collections::HasKey, ui::ToRow};

#[derive(Copy, Clone, Debug, Default, Deserialize, Serialize)]
pub struct SystemNotificationData {
    pub timestamp: Nanos,
    pub notification: SystemNotification,
}
impl SystemNotificationData {
    pub fn new(timestamp: Nanos, notification: SystemNotification) -> Self {
        Self { timestamp, notification }
    }
}

impl HasKey for SystemNotificationData {
    type Key = Nanos;

    fn key(&self) -> &Self::Key {
        &self.timestamp
    }
}

impl ToRow for SystemNotificationData {
    fn to_row<'a>(&'a self, _data: &Data) -> Vec<Text<'a>> {
        vec![self.timestamp.with_fmt("%d %H:%M:%S%.3f").into(), format!("{:?}", self.notification).into()]
    }
}
