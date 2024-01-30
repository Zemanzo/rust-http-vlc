use futures_util::{SinkExt, StreamExt};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use tokio::sync::mpsc;
use warp::filters::ws::{Message, WebSocket};
use warp::Filter;

fn start_vlc_thread(sender: mpsc::Sender<String>, receiver: mpsc::Receiver<Message>) {
    let vlc_path = Path::new("vlc");
    let child = Command::new(vlc_path)
        .arg("-I")
        .arg("rc")
        .arg("--no-rc-fake-tty")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("failed to start vlc");

    println!("Started process: {}", child.id());

    thread::spawn(move || async {
        let mut vlc_output = BufReader::new(child.stdout.unwrap());
        let mut stdin = child.stdin.unwrap();
        while let Some(msg) = receiver.recv().await {
            stdin.write_all(msg.as_bytes()).unwrap();
            let mut buf = String::new();
            match vlc_output.read_line(&mut buf) {
                Ok(_) => {
                    sender.send(buf).await.unwrap();
                    continue;
                }
                Err(e) => {
                    println!("an error!: {:?}", e);
                    break;
                }
            }
        }
    });
}

#[tokio::main]
async fn main() {
    if cfg!(target_os = "windows") {
        println!("This application is not supported on Windows: VLC rc does not support stdin input on Windows.");
    }
    let (input_sender, input_receiver) = mpsc::channel(100);
    let (output_sender, output_receiver) = mpsc::channel(100);

    let output_receiver = Arc::new(Mutex::new(output_receiver));

    start_vlc_thread(output_sender, input_receiver);

    // GET /
    let root = warp::path::end().map(|| "Hello, World at root!");

    let socket = warp::path("socket")
        // The `ws()` filter will prepare the Websocket handshake.
        .and(warp::ws())
        .map(|ws: warp::ws::Ws| {
            // And then our closure will be called when it completes...
            ws.on_upgrade(move |websocket| {
                socket_handler(websocket, output_receiver, input_sender.clone())
            })
        });

    let routes = warp::get().and(root.or(socket));

    warp::serve(routes).run(([127, 0, 0, 1], 3030)).await;
}

async fn socket_handler(
    ws: WebSocket,
    output_receiver: Arc<Mutex<mpsc::Receiver<String>>>,
    input_sender: mpsc::Sender<Message>,
) {
    let (mut socket_sender, mut socket_receiver) = ws.split();
    tokio::task::spawn(async move {
        let mut rec = output_receiver.lock().unwrap();
        while let Some(line) = rec.recv().await {
            socket_sender.send(Message::text(line));
        }
    });
    while let Some(msg) = socket_receiver.next().await {
        input_sender.send(msg.unwrap());
    }
}
