use bop_common::{actor::Actor, communication::SpineConnections, p2p, signing::ECDSASigner};
use jsonrpsee::client_transport::ws::Url;
use reqwest::blocking::{Client, ClientBuilder};
use tracing::{error, info};

pub struct Gossiper {
    target_rpc: Option<Url>,
    client: Client,
    signer: ECDSASigner,
}

impl Gossiper {
    pub fn new(target_rpc: Option<Url>) -> Self {
        let client = ClientBuilder::new()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("couldn't build http client");

        let signer = ECDSASigner::random();

        Self { target_rpc, client, signer }
    }

    fn gossip(&self, msg: p2p::VersionedMessage) {
        let Some(url) = self.target_rpc.as_ref().cloned() else {
            return;
        };

        let payload = msg.to_json(&self.signer);

        let Ok(res) = self.client.post(url).json(&payload).send() else {
            tracing::error!("couldn't send {}", payload);
            return;
        };

        let code = res.status();
        let body = res.text().expect("couldn't read response");

        if code.is_success() {
            info!("successfully sent {}", msg.as_ref());
        } else {
            error!(body, %payload, code = code.as_u16(), "failed to send");
        }
    }
}

impl<Db> Actor<Db> for Gossiper {
    fn loop_body(&mut self, connections: &mut SpineConnections<Db>) {
        connections.receive(|msg, _| {
            self.gossip(msg);
        });
    }
}
