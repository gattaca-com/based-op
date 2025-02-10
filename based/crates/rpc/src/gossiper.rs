use bop_common::{actor::Actor, communication::SpineConnections, p2p};
use jsonrpsee::client_transport::ws::Url;
use reqwest::blocking::{Client, ClientBuilder};

pub struct Gossiper {
    target_rpc: Option<Url>,
    client: Client,
}
impl Gossiper {
    pub fn new(target_rpc: Option<Url>) -> Self {
        let client = ClientBuilder::new()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("couldn't build http client");
        Self { target_rpc, client }
    }

    fn gossip(&self, msg: p2p::VersionedMessage) {
        let Some(url) = self.target_rpc.as_ref().cloned() else {
            return;
        };
        let Ok(res) = self.client.post(url).json(&msg).send() else {
            tracing::error!("couldn't send {}", msg.as_ref());
            return;
        };

        if let Err(e) = res.error_for_status_ref() {
            tracing::error!("received {e} upon sending {msg:?}");
            tracing::error!("body: {:.20}", format!("{:?}", res.text()));
        }
        tracing::info!(" successfully sent {}", msg.as_ref());
    }
}

impl<Db> Actor<Db> for Gossiper {
    fn loop_body(&mut self, connections: &mut SpineConnections<Db>) {
        connections.receive(|msg, _| {
            self.gossip(msg);
        });
    }
}
