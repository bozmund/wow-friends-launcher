use crate::config::Invitation;
use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::watch,
    task::JoinSet,
};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

pub struct Relay {
    cancel: watch::Sender<bool>,
    pub connections: Arc<AtomicUsize>,
}
impl Drop for Relay {
    fn drop(&mut self) {
        let _ = self.cancel.send(true);
    }
}
impl Relay {
    pub async fn start(invite: Invitation) -> Result<Self> {
        invite.validate()?;
        let mut listeners = vec![];
        // Bind all ports before spawning any task; a failure releases all listeners.
        for port in [1119u16, 18081, 18085] {
            listeners.push((port,TcpListener::bind((std::net::Ipv4Addr::LOCALHOST,port)).await.with_context(||format!("Local port {port} is already in use. Close the old launcher before playing."))?));
        }
        let (cancel, rx) = watch::channel(false);
        let connections = Arc::new(AtomicUsize::new(0));
        for (port, listener) in listeners {
            let invite = invite.clone();
            let mut stop = rx.clone();
            let count = connections.clone();
            tokio::spawn(async move {
                let mut tasks = JoinSet::new();
                loop {
                    tokio::select! {
                        _=stop.changed()=>break,
                        accepted=listener.accept()=>match accepted {Ok((stream,_))=>{let invite=invite.clone();let count=count.clone();let stop=stop.clone();tasks.spawn(async move {let _=pump(stream,&invite,port,stop,count).await;});},Err(_)=>break},
                        Some(_)=tasks.join_next(),if !tasks.is_empty()=>{}
                    }
                }
                tasks.abort_all();
                while tasks.join_next().await.is_some() {}
            });
        }
        Ok(Self {
            cancel,
            connections,
        })
    }
}
async fn pump(
    mut stream: TcpStream,
    invite: &Invitation,
    port: u16,
    mut stop: watch::Receiver<bool>,
    count: Arc<AtomicUsize>,
) -> Result<()> {
    stream.set_nodelay(true)?;
    if count.fetch_add(1, Ordering::SeqCst) >= 16 {
        count.fetch_sub(1, Ordering::SeqCst);
        return Ok(());
    }
    struct Counter(Arc<AtomicUsize>);
    impl Drop for Counter {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }
    let _guard = Counter(count);
    let origin = invite.origin().replacen("https://", "wss://", 1);
    let mut request = format!("{origin}/connect/{port}").into_client_request()?;
    request
        .headers_mut()
        .insert("Authorization", format!("Bearer {}", invite.token).parse()?);
    let (socket, _) = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        tokio_tungstenite::connect_async_with_config(request, None, true),
    )
    .await??;
    let (mut output, mut input) = socket.split();
    let (mut read, mut write) = stream.split();
    let up = async {
        let mut data = vec![0u8; 65536];
        loop {
            let n = read.read(&mut data).await?;
            if n == 0 {
                break;
            }
            output
                .send(Message::Binary(data[..n].to_vec().into()))
                .await?;
        }
        Ok::<_, anyhow::Error>(())
    };
    let down = async {
        while let Some(message) = input.next().await {
            match message? {
                Message::Binary(data) => write.write_all(&data).await?,
                Message::Close(_) => break,
                Message::Text(_) => break,
                _ => {}
            }
        }
        Ok::<_, anyhow::Error>(())
    };
    tokio::select! {r=up=>r?,r=down=>r?,_=stop.changed()=>{}}
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn binary_stream_survives_fragmentation_and_cancels() {
        let remote = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = remote.local_addr().unwrap();
        let echo = tokio::spawn(async move {
            let (stream, _) = remote.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            while let Some(Ok(m)) = ws.next().await {
                if let Message::Binary(b) = m {
                    for chunk in b.chunks(333) {
                        ws.send(Message::Binary(chunk.to_vec().into()))
                            .await
                            .unwrap();
                    }
                }
            }
        });
        let local = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = TcpStream::connect(local.local_addr().unwrap())
            .await
            .unwrap();
        let (accepted, _) = local.accept().await.unwrap();
        let (stop, rx) = watch::channel(false);
        let counter = Arc::new(AtomicUsize::new(0));
        let invite = Invitation {
            name: "Fixture".into(),
            account: "fixture@local.invalid".into(),
            token: "fixture-only".into(),
            relay: format!("ws://{addr}"),
            password: None,
            ca_pem: String::new(),
            allowed_builds: vec![],
        };
        let c = counter.clone();
        let bridge = tokio::spawn(async move { pump(accepted, &invite, 18085, rx, c).await });
        let bytes: Vec<u8> = (0..200000).map(|n| (n % 251) as u8).collect();
        client.write_all(&bytes).await.unwrap();
        let mut got = vec![0; bytes.len()];
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            client.read_exact(&mut got),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(bytes, got);
        stop.send(true).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), bridge)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(counter.load(Ordering::SeqCst), 0);
        echo.abort();
    }
}
