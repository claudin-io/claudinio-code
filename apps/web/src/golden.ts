/// A recorded exchange with the real device, in hex.
///
/// Produced by `print_golden_handshake` in `src-tauri/src/remote/noise.rs`, with both
/// Noise ephemerals fixed so the whole transcript is reproducible:
///
/// ```sh
/// cd src-tauri
/// cargo test --features remote golden -- --ignored --nocapture
/// ```
///
/// # Why this file exists
///
/// Every other test in this app checks the browser against itself. These bytes are
/// the only thing that can show the browser and the device agree — and the two have
/// already disagreed once in a way no unit test could have caught: the device
/// attached to the relay without a channel token, so nothing could ever have
/// connected, while both sides' suites stayed green.
///
/// The static keys are regenerated on every run, so every constant here has to be
/// replaced as a set. Do not hand-edit one of them.
export const GOLDEN = {
  initStaticPrivate: "30bd0b49ee78b00122756fab92724c78d31b6440b9eb0815f4cf7e48e6116b29",
  initStaticPublic: "eac77fc7d5394475edc71f7fdd8a8ae12b388d90716891b494a1f87470a07c46",
  initEphemeralPrivate: "0202020202020202020202020202020202020202020202020202020202020202",
  respStaticPublic: "0d8df651436e03bc8c6857903f101b39bd6b0026a40752246b9c95f320134f37",

  msg1: "ce8d3ad1ccb633ec7b70c17814a5c76ecd029685050d344745ba05870e587d59b6d1d453a65724cfe761551d6ee7e9ad8a45f70465d234ea400dcef17f7ea6f26bae1d16520ab4bb2cae55479ccbfd4558c77e7aa42981721e46ec3fd1adfb2a",
  msg2: "ac01b2209e86354fb853237b5de0f4fab13c7fcbf433a61c019369617fecf10ba3fe4bb95ac30b2b4e7e46fb4eedfbd2",
  handshakeHash: "ce456ac7445fb54adc6fb96b753f5d7cf9a65651bff3078e72f6dfa06f8ca999",
  sas: "fathom · harbour · flint",

  /// Bare transport ciphertext, for the cipher-state tests.
  deviceCiphertext: "99a25bca9c1eb6ff446aa4cbdf64c0ae5add417c2955b5176d5bc5687e3eec5b62a279ef48",
  devicePlaintext: "hello from the device",
  peerCiphertext: "0069e12488ff4eb2d2a537ad2b2c39893bec61c7aacf4786ee09a644a251e3a244f7ab25ed24",
  peerPlaintext: "hello from the browser",

  /// Whole frames, as they go on the wire. The channel is 0xab repeated.
  ///
  /// Taken from a *second* handshake, so they start at cipher counter zero and can be
  /// used without first consuming the bare vectors above. A Noise nonce advances per
  /// message, so a vector's position in the stream is part of it — with the ephemerals
  /// fixed the two handshakes are identical, which is what makes this sound rather
  /// than a fudge.
  ///
  /// Within this set the order still matters: snapshot, then close.
  channel: "ab".repeat(16),
  snapshotFrame:
    "86a17601a46b696e6403a76368616e6e656cc410ababababababababababababababababa373657102a361636b00a77061796c6f6164c47475635ccf9d5a78fe4566f4ccdf6e94604cce5b6c2549dfd1c2112120ead4e894108c4d326d01a550148c054875416d1bb426bce98771ffbeaafc6d16681728b6921e55576e407a497da11b8ea2c32fa7fab622ec16ee25821caa6b13567207b05e122465eca058df8467feb3cea8270fe9c3dafa",
  closedFrame:
    "86a17601a46b696e6403a76368616e6e656cc410ababababababababababababababababa373657103a361636b00a77061796c6f6164c4379fc14fe608d896690f4ad73d169dd98b514806241aad947a9120679ca858b40f62da1e25dd2fb6d45d599f48377b81387f06626c605111",
  /// What the browser has to be able to *send*: a `Subscribe` for session "golden"
  /// from seq 0, at frame seq 1.
  subscribeFrame:
    "86a17601a46b696e6403a76368616e6e656cc410ababababababababababababababababa373657101a361636b00a77061796c6f6164c43ceba8e62189bb81b3c8aa64ba31207b8ee3f073c7bcd4fe8c16175f853041c7a19124581c1624274fc2aded9d4b6e3ef505312b520914b95b479c0387",
} as const;
