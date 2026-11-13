//! The protocol round trips, survives any way of cutting the stream, and rejects bad input
//! with an error instead of stopping the program.

use arcade_proto::*;
use arcade_workload::Rng;

fn client_messages() -> Vec<ClientMessage> {
    vec![
        ClientMessage::Hello {
            version: PROTOCOL_VERSION,
        },
        ClientMessage::Attach(AttachRequest {
            session: Some("work".into()),
            create: true,
            new_name: Some("new one".into()),
            command: vec!["nvim".into(), "a b.txt".into()],
            cwd: "/tmp/dir with space".into(),
            env: vec!["A=1".into(), "TERM=xterm-256color".into()],
            cols: 200,
            rows: 50,
        }),
        ClientMessage::Attach(AttachRequest {
            session: None,
            create: false,
            new_name: None,
            command: vec![],
            cwd: String::new(),
            env: vec![],
            cols: 1,
            rows: 1,
        }),
        ClientMessage::Input(vec![]),
        ClientMessage::Input(b"ls -la\r\x1b[A\x00\xff".to_vec()),
        ClientMessage::Resize {
            cols: 65535,
            rows: 0,
        },
        ClientMessage::Detach,
        ClientMessage::Command {
            name: "session.detach".into(),
            args: vec![],
        },
        ClientMessage::Command {
            name: "x".into(),
            args: vec!["tiếng Việt".into(), "汉字".into(), "👨\u{200D}👩".into()],
        },
    ]
}

fn server_messages() -> Vec<ServerMessage> {
    vec![
        ServerMessage::HelloOk {
            version: PROTOCOL_VERSION,
        },
        ServerMessage::VersionMismatch {
            daemon: 7,
            client: 9,
        },
        ServerMessage::Attached {
            session: "main".into(),
        },
        ServerMessage::Output(vec![]),
        ServerMessage::Output((0..=255u8).cycle().take(100_000).collect()),
        ServerMessage::Detached {
            reason: DetachReason::Replaced,
            message: "another client attached".into(),
        },
        ServerMessage::Detached {
            reason: DetachReason::Requested,
            message: String::new(),
        },
        ServerMessage::Detached {
            reason: DetachReason::ServerStopping,
            message: "server stopping".into(),
        },
        ServerMessage::SessionEnded {
            message: "exited".into(),
        },
        ServerMessage::CommandResult {
            ok: true,
            output: "a\nb\n".into(),
        },
        ServerMessage::CommandResult {
            ok: false,
            output: "no such session".into(),
        },
    ]
}

/// Decode a whole stream; the stream must end exactly on a message boundary.
fn decode_all<M: Message>(bytes: &[u8], chunk: usize) -> Result<Vec<M>, ProtoError> {
    let (out, left) = decode_prefix::<M>(bytes, chunk)?;
    assert_eq!(left, 0, "bytes left over");
    Ok(out)
}

/// Decode what can be decoded; also say how many bytes were left unread (a cut-off tail).
fn decode_prefix<M: Message>(bytes: &[u8], chunk: usize) -> Result<(Vec<M>, usize), ProtoError> {
    let mut decoder = Decoder::<M>::new();
    let mut out = Vec::new();
    for piece in bytes.chunks(chunk.max(1)) {
        decoder.push(piece);
        while let Some(msg) = decoder.next_message()? {
            out.push(msg);
        }
    }
    Ok((out, decoder.buffered()))
}

#[test]
fn every_client_message_round_trips() {
    for msg in client_messages() {
        let bytes = to_bytes(&msg).unwrap();
        assert_eq!(
            decode_all::<ClientMessage>(&bytes, usize::MAX).unwrap(),
            vec![msg]
        );
    }
}

#[test]
fn every_server_message_round_trips() {
    for msg in server_messages() {
        let bytes = to_bytes(&msg).unwrap();
        assert_eq!(
            decode_all::<ServerMessage>(&bytes, usize::MAX).unwrap(),
            vec![msg]
        );
    }
}

#[test]
fn several_messages_in_one_stream_come_out_in_order() {
    let msgs = client_messages();
    let mut bytes = Vec::new();
    for m in &msgs {
        encode(m, &mut bytes).unwrap();
    }
    assert_eq!(
        decode_all::<ClientMessage>(&bytes, usize::MAX).unwrap(),
        msgs
    );
}

#[test]
fn stream_cut_at_every_position_still_decodes() {
    let msgs = client_messages();
    let mut bytes = Vec::new();
    for m in &msgs {
        encode(m, &mut bytes).unwrap();
    }
    // Two pieces, cut at every byte position.
    for cut in 0..=bytes.len() {
        let mut decoder = Decoder::<ClientMessage>::new();
        let mut out = Vec::new();
        for piece in [&bytes[..cut], &bytes[cut..]] {
            decoder.push(piece);
            while let Some(m) = decoder.next_message().unwrap() {
                out.push(m);
            }
        }
        assert_eq!(out, msgs, "cut at {cut}");
    }
    // And one byte at a time, and in odd sized pieces.
    for chunk in [1, 2, 3, 7, 13, 1000] {
        assert_eq!(
            decode_all::<ClientMessage>(&bytes, chunk).unwrap(),
            msgs,
            "chunk {chunk}"
        );
    }
}

#[test]
fn server_stream_cut_everywhere_still_decodes() {
    let msgs = server_messages();
    let mut bytes = Vec::new();
    for m in &msgs {
        encode(m, &mut bytes).unwrap();
    }
    for chunk in [1, 5, 4096, 65_537] {
        assert_eq!(
            decode_all::<ServerMessage>(&bytes, chunk).unwrap(),
            msgs,
            "chunk {chunk}"
        );
    }
}

#[test]
fn incomplete_message_waits_for_more() {
    let bytes = to_bytes(&ClientMessage::Input(b"abcdef".to_vec())).unwrap();
    let mut decoder = Decoder::<ClientMessage>::new();
    decoder.push(&bytes[..bytes.len() - 1]);
    assert_eq!(decoder.next_message().unwrap(), None);
    decoder.push(&bytes[bytes.len() - 1..]);
    assert!(decoder.next_message().unwrap().is_some());
}

#[test]
fn oversized_length_is_refused_at_once_without_waiting_for_the_bytes() {
    let mut decoder = Decoder::<ClientMessage>::new();
    // Only the four length bytes arrive; they announce 4 GiB - 1.
    decoder.push(&u32::MAX.to_le_bytes());
    assert_eq!(
        decoder.next_message(),
        Err(ProtoError::TooLarge {
            len: u32::MAX as usize
        })
    );
    // The decoder stays failed and holds nothing.
    assert!(decoder.next_message().is_err());
    assert_eq!(decoder.buffered(), 0);
    decoder.push(&[0; 100]);
    assert_eq!(decoder.buffered(), 0);
}

#[test]
fn message_just_over_the_limit_is_refused_and_one_at_the_limit_is_not() {
    let mut decoder = Decoder::<ClientMessage>::new();
    decoder.push(&((MAX_MESSAGE_LEN + 1) as u32).to_le_bytes());
    assert!(matches!(
        decoder.next_message(),
        Err(ProtoError::TooLarge { .. })
    ));

    // Encoding refuses too, and leaves the buffer as it was.
    let mut out = b"keep".to_vec();
    let too_big = ClientMessage::Input(vec![0; MAX_MESSAGE_LEN]);
    assert!(matches!(
        encode(&too_big, &mut out),
        Err(ProtoError::TooLarge { .. })
    ));
    assert_eq!(out, b"keep");

    let biggest = ClientMessage::Input(vec![7; MAX_MESSAGE_LEN - 1]);
    let bytes = to_bytes(&biggest).unwrap();
    assert_eq!(
        decode_all::<ClientMessage>(&bytes, 1 << 20).unwrap(),
        vec![biggest]
    );
}

#[test]
fn zero_length_unknown_tag_and_bad_payloads_are_errors() {
    let mut d = Decoder::<ClientMessage>::new();
    d.push(&0u32.to_le_bytes());
    assert_eq!(d.next_message(), Err(ProtoError::Truncated));

    let mut d = Decoder::<ClientMessage>::new();
    d.push(&[1, 0, 0, 0, 200]);
    assert_eq!(d.next_message(), Err(ProtoError::UnknownTag(200)));

    // Resize with only three of its four bytes.
    let mut d = Decoder::<ClientMessage>::new();
    d.push(&[4, 0, 0, 0, 4, 1, 2, 3]);
    assert_eq!(d.next_message(), Err(ProtoError::Truncated));

    // Detach with a stray byte.
    let mut d = Decoder::<ClientMessage>::new();
    d.push(&[2, 0, 0, 0, 5, 9]);
    assert_eq!(d.next_message(), Err(ProtoError::TrailingBytes));

    // A command whose name is not UTF-8.
    let mut d = Decoder::<ClientMessage>::new();
    d.push(&[10, 0, 0, 0, 6, 1, 0, 0, 0, 0xff, 0, 0, 0, 0]);
    assert_eq!(d.next_message(), Err(ProtoError::InvalidUtf8));

    // A string announcing far more bytes than the message holds allocates nothing and fails.
    let mut d = Decoder::<ClientMessage>::new();
    d.push(&[9, 0, 0, 0, 6, 0xff, 0xff, 0xff, 0x7f, 0, 0, 0, 0]);
    assert_eq!(d.next_message(), Err(ProtoError::Truncated));

    // A list announcing millions of items is refused before any of them is read.
    let mut d = Decoder::<ClientMessage>::new();
    d.push(&[10, 0, 0, 0, 6, 1, 0, 0, 0, b'x', 0xff, 0xff, 0xff, 0x00]);
    assert_eq!(d.next_message(), Err(ProtoError::ListTooLong));

    // A boolean that is neither 0 nor 1.
    let mut d = Decoder::<ServerMessage>::new();
    d.push(&[6, 0, 0, 0, 7, 2, 0, 0, 0, 0]);
    assert_eq!(d.next_message(), Err(ProtoError::InvalidValue));
}

#[test]
fn random_bytes_never_stop_the_program() {
    let mut rng = Rng::new(2026);
    for round in 0..3000 {
        let len = rng.range(0, 300);
        let mut data = vec![0u8; len];
        rng.fill(&mut data);
        // Make some rounds look more like real messages: a small, plausible length and tag.
        if round % 2 == 0 && len >= 5 {
            data[0] = (len - 4) as u8;
            data[1] = 0;
            data[2] = 0;
            data[3] = 0;
            data[4] = rng.range(1, 8) as u8;
        }
        let chunk = rng.range(1, 40);
        let _ = decode_prefix::<ClientMessage>(&data, chunk);
        let _ = decode_prefix::<ServerMessage>(&data, chunk);
    }
}

#[test]
fn flipping_any_single_byte_of_a_valid_stream_never_stops_the_program() {
    let mut bytes = Vec::new();
    for m in client_messages() {
        encode(&m, &mut bytes).unwrap();
    }
    for i in 0..bytes.len() {
        for flip in [0x01u8, 0x80, 0xff] {
            let mut copy = bytes.clone();
            copy[i] ^= flip;
            let _ = decode_prefix::<ClientMessage>(&copy, 11);
        }
    }
}

#[test]
fn different_versions_give_an_error_carrying_both_numbers() {
    assert_eq!(check_version(3, 3), Ok(()));
    let err = check_version(3, 5).unwrap_err();
    assert_eq!(
        err,
        ProtoError::VersionMismatch {
            daemon: 3,
            client: 5
        }
    );
    let text = err.to_string();
    assert!(text.contains('3') && text.contains('5'), "{text}");
}

#[test]
fn hello_and_version_mismatch_keep_their_layout() {
    // These bytes must never change, or two versions could not tell each other apart.
    assert_eq!(
        to_bytes(&ClientMessage::Hello {
            version: 0x0403_0201
        })
        .unwrap(),
        vec![5, 0, 0, 0, 1, 1, 2, 3, 4]
    );
    assert_eq!(
        to_bytes(&ServerMessage::VersionMismatch {
            daemon: 1,
            client: 2
        })
        .unwrap(),
        vec![9, 0, 0, 0, 2, 1, 0, 0, 0, 2, 0, 0, 0]
    );
}

#[test]
fn a_hello_of_any_other_version_still_decodes() {
    for version in [0, 1, 2, 99, u32::MAX] {
        let bytes = to_bytes(&ClientMessage::Hello { version }).unwrap();
        assert_eq!(
            decode_all::<ClientMessage>(&bytes, 3).unwrap(),
            vec![ClientMessage::Hello { version }]
        );
    }
}
