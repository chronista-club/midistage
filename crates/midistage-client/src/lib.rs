pub use midistage_protocol::{self as protocol, wire::*};
use unison::network::{ProtocolClient, QuicClient, TrustAnchors, channel::UnisonChannel};

#[derive(Debug, thiserror::Error)]
#[error("{code}: {message}")]
pub struct RemoteError {
    pub code: String,
    pub message: String,
}

pub struct Client {
    transport: ProtocolClient,
    channel: UnisonChannel,
}
impl Client {
    /// 版違いの常駐サービスを停止・上書きしない。再接続と UI はアプリの責務。
    pub async fn connect(
        endpoint: &Endpoint,
        mut hello: Hello,
    ) -> anyhow::Result<(Self, Snapshot)> {
        anyhow::ensure!(
            endpoint.protocol_version == protocol::PROTOCOL_VERSION,
            "incompatible midistage protocol"
        );
        let quic = QuicClient::builder()
            .trust_anchors(TrustAnchors::Custom(vec![
                endpoint.certificate_der.clone().into(),
            ]))
            .build()?;
        let transport = ProtocolClient::new(quic);
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            transport.connect(&format!("https://localhost:{}", endpoint.port)),
        )
        .await??;
        let channel = match tokio::time::timeout(
            std::time::Duration::from_secs(3),
            transport.open_channel(CHANNEL),
        )
        .await
        {
            Ok(Ok(channel)) => channel.with_request_timeout(std::time::Duration::from_secs(3)),
            Ok(Err(error)) => {
                let _ = transport.disconnect().await;
                return Err(error.into());
            }
            Err(error) => {
                let _ = transport.disconnect().await;
                return Err(error.into());
            }
        };
        let client = Self { transport, channel };
        hello.auth_token = endpoint.auth_token.clone();
        let result = client.request("Hello", &hello).await;
        match result {
            Ok(snapshot) if snapshot.server_epoch == endpoint.server_epoch => {
                Ok((client, snapshot))
            }
            result => {
                let _ = client.close().await;
                match result {
                    Err(error) => Err(error),
                    Ok(_) => anyhow::bail!("midistage server epoch changed"),
                }
            }
        }
    }
    pub async fn set_enabled(&self, request: &SetEnabled) -> anyhow::Result<Snapshot> {
        self.request("SetEnabled", request).await
    }
    /// SysEx は物理ドライバの completion まで待つ。
    pub async fn send_midi(&self, request: &SendMidi) -> anyhow::Result<Snapshot> {
        self.request("SendMidi", request).await
    }
    pub async fn snapshot(&self) -> anyhow::Result<Snapshot> {
        self.request("Snapshot", &serde_json::json!({})).await
    }
    /// アプリが入力停止・発音整理・出力キュー停止を終えてから明示的に呼ぶ。
    pub async fn quiesced(&self, request: &Quiesced) -> anyhow::Result<Snapshot> {
        self.request("Quiesced", request).await
    }
    pub async fn recv_event(&self) -> anyhow::Result<Event> {
        let message = self.channel.recv().await?;
        Ok(serde_json::from_value(message.payload_as_value()?)?)
    }
    pub async fn close(&self) -> anyhow::Result<()> {
        let _ = self.channel.close().await;
        self.transport.disconnect().await?;
        Ok(())
    }
    async fn request<T: serde::Serialize>(
        &self,
        method: &str,
        request: &T,
    ) -> anyhow::Result<Snapshot> {
        let reply: Reply = self.channel.request(method, request).await?;
        if let Some(error) = reply.error {
            return Err(RemoteError {
                code: error.code,
                message: error.message,
            }
            .into());
        }
        reply
            .snapshot
            .ok_or_else(|| anyhow::anyhow!("midistage reply missing snapshot"))
    }
}
