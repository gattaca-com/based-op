use bop_common::{
    actor::Actor,
    communication::SpineConnections,
    p2p::{self, FragV0, SignedVersionedMessage},
    signing::ECDSASigner,
};
use jsonrpsee::client_transport::ws::Url;
use reqwest::blocking::{Client, ClientBuilder};
use tracing::{error, info};

pub struct Gossiper {
    target_rpc: Url,
    client: Client,
    signer: ECDSASigner,
    frag_broadcast: tokio::sync::broadcast::Sender<SignedVersionedMessage>,
}

impl Gossiper {
    pub fn new(
        target_rpc: Url,
        signer: Option<ECDSASigner>,
        frag_broadcast: tokio::sync::broadcast::Sender<SignedVersionedMessage>,
    ) -> Self {
        let client = ClientBuilder::new()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("couldn't build http client");

        let signer = signer.unwrap_or_else(ECDSASigner::random);

        Self { target_rpc, client, signer, frag_broadcast }
    }

    fn gossip(&self, msg: p2p::VersionedMessage) {
        let signed = msg.to_signed(&self.signer);
        let payload = signed.to_json();

        let Ok(res) = self.client.post(self.target_rpc.clone()).json(&payload).send() else {
            tracing::error!("couldn't send {}", payload);
            return;
        };

        let code = res.status();
        let body = res.text().expect("couldn't read response");

        if code.is_success() {
            info!("successfully sent {:?}", signed);
        } else {
            error!(body, %payload, code = code.as_u16(), "failed to send");
        }
        if matches!(signed.message, p2p::VersionedMessage::FragV0(_)) {
            let mut to_send = signed;
            while let Err(e) = self.frag_broadcast.send(to_send) {
                to_send = e.0;
            }
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
