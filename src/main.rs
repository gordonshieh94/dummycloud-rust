use std::env;
use std::net::UdpSocket;
use std::time::SystemTime;

use bytes::{Buf, BufMut, BytesMut};
use getopts::Options;
use serde_json::json;

mod codec;
mod payload;

fn create_timesync_packet() -> BytesMut {
    let mut packet = BytesMut::with_capacity(32);

    // byte 0: set headers for sending timesync packet
    packet.put_u8(0x21);
    packet.put_u8(0x31);

    // byte 2: size of the timestamp field
    packet.put_u8(0x00);
    packet.put_u8(0x20);

    // byte 4 - 11: emptiness
    packet.put_slice(&[0xff; 8]);

    let epoch = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs() as u32;

    // byte 12-15: write the epoch timestamp, yes this will fail in 2038.
    packet.put_u32(epoch);

    // fill the rest of the 32 byte packet with emptiness
    packet.put_slice(&[0xff; 16]);
    packet
}

fn print_usage(program: &str, opts: Options) {
    let brief = format!("Usage: {} abcdef [options]", program);
    print!("{}", opts.usage(&brief));
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = env::args().collect();

    let mut opts = Options::new();
    opts.opt(
        "k",
        "key",
        "Cloud key used to identify your robot to Xiaomi.",
        "SoMeALPhaCHars",
        getopts::HasArg::Yes,
        getopts::Occur::Req,
    );

    let matches = match opts.parse(&args[1..]) {
        Ok(m) => m,
        Err(_) => {
            print_usage(&args[0].clone(), opts);
            std::process::exit(1);
        }
    };
    let cloud_key = match matches.opt_str("k") {
        Some(s) => s,
        None => {
            print_usage(&args[0].clone(), opts);
            std::process::exit(1);
        }
    };

    let socket = UdpSocket::bind("0.0.0.0:8053").expect("Could not bind to address");
    println!("Dummycloud is now listening");

    loop {
        let mut buf = [0; 1024];
        let (amt, src) = socket.recv_from(&mut buf)?;
        println!("connected from: {} with a message of length: {}", src, amt);

        let c = codec::UDPCodec::new(&cloud_key);

        // truncate the size of the buffer to appropriately handle later
        let buf = &buf[..amt];

        let header = &buf[..32];
        let encrypted_body = &buf[32..];
        let stamp = (&header[12..]).get_u32();
        let device_id = (&header[8..]).get_u32();
        let response = match c.decode_response(header, encrypted_body) {
            Some(s) => s,
            None => {
                if stamp == 0 {
                    println!("Robot connected!");
                    socket.send_to(create_timesync_packet().bytes(), &src)?;
                } else {
                    socket.send_to(&buf, &src)?;
                }
                continue;
            }
        };

        let response: payload::MessagePayload = match serde_json::from_str(&response) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let response_function = response.method.as_str();
        let reply_json: payload::ResponsePayload = match response_function {
            "_otc.info" => payload::ResponsePayload::new(
                response.id,
                json!({
                    "otc_list": [{
                        "ip": "130.83.47.181",
                        "port": 8053
                    }
                    ],
                    "otc_test": {
                        "list": [{
                            "ip": "130.83.47.181",
                            "port": 8053
                        }
                        ],
                        "interval": 1800,
                        "firsttest": 1193
                    }
                }),
            ),
            "props" | "event.status" | "event.low_power_back" => {
                payload::ResponsePayload::new(response.id, serde_json::to_value("ok")?)
            }
            "_sync.gen_presigned_url" => payload::ResponsePayload::new(
                response.id,
                json!({"" : { "url": "http://us.ott.io.mi.com/robomap", "obj_name": "something", "method": "PUT",
                     "expires_time": (SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs() + 3600),
                        "ok": true,
                        "pwd": "password"
                }}),
            ),
            "_sync.batch_gen_room_up_url" => payload::ResponsePayload::new(
                response.id,
                json!([
                    "http://us.ott.io.mi.com/robomap/1",
                    "http://us.ott.io.mi.com/robomap/2",
                    "http://us.ott.io.mi.com/robomap/3",
                    "http://us.ott.io.mi.com/robomap/4"
                ]),
            ),
            _ => {
                println!("unknown event: {}", response_function);
                continue;
            }
        };
        let reply = c.encode_response(&serde_json::to_vec(&reply_json)?, device_id);
        socket.send_to(&reply, &src)?;
    }
}
