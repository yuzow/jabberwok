# Jabberwok

A macOS speech-to-text service that transcribes your voice wherever your cursor
is. Hold the hotkey, speak, release and your words appear in any focused input.

**100% local and offline.** Audio never leaves your machine.

<!-- TODO: add a demo GIF here -->

## Why Jabberwok

- **Universal** — works in any app across the entire OS with a focused text
  input, including terminals, editors, and browsers
- **Instant** — no cloud round-trip; transcription runs on-device with a
  quantized ONNX model at 30X real time speed
- **Low friction** — one held key to record, one release to inject; no window
  switching, no copy-paste
- **macOS-native** — integrates with LaunchAgents, app bundles, and the
  Accessibility API

## Installation (macOS)

### Download and install

1. Download the latest `Jabberwok.dmg` from the [Releases](../../releases) page.
2. Open the DMG, drag **Jabberwok.app** to your `Applications` folder.
3. Launch Jabberwok — the first-run setup window will guide you through
   downloading the default model and granting the required permissions.
4. Optionally add the CLI to your PATH to be able to run CLI commands:
   ```sh
   ln -s /Applications/Jabberwok.app/Contents/MacOS/jabberwok /usr/local/bin/jabberwok
   ```

### Homebrew

> Coming soon.

### Rust Crate

> Coming soon.

## Getting Started

Once installed, launch Jabberwok from your Applications folder or Spotlight. The
first-run setup window walks you through the two required steps:

1. **Download the default model** — a quantized on-device speech model (~few
   hundred MB).
2. **Grant permissions** — Microphone and Accessibility (see
   [Permissions](#permissions) below).

After setup, Jabberwok runs as a background daemon. To use it:

- **Hold Right Command** to start recording.
- **Speak.**
- **Release Right Command** — the transcribed text is injected at your cursor.

Use `doctor` to check that everything is configured correctly at any time:

```sh
jabberwok doctor
```

> **Developers:** to run from source without installing, see
> [CONTRIBUTING.md](CONTRIBUTING.md).

## Permissions

Jabberwok requires two macOS permissions:

| Permission        | Why                                                          |
| ----------------- | ------------------------------------------------------------ |
| **Microphone**    | To record your voice                                         |
| **Accessibility** | To inject transcribed text into the focused input of any app |

Neither permission is used for anything other than local recording and local
text injection.

### App vs. terminal

Which app macOS prompts depends on how you launch Jabberwok:

- **Installed app (DMG)** — Jabberwok.app itself requests both permissions.
  Grant them to **Jabberwok** in System Settings → Privacy & Security.
- **Command line** — the _terminal emulator_ you are running in (Terminal,
  WezTerm, Kitty, Ghostty, etc.) requests both permissions on Jabberwok's
  behalf. Grant them to **your terminal app** in System Settings → Privacy &
  Security. If you switch terminal apps later, you may need to re-grant.

Run `jabberwok permissions all` (or `cargo run -- permissions all` from source)
to open the relevant system settings dialogs directly.

## Models

> **Default model:** Parakeet v3 (quantized int8 ONNX). Downloaded automatically
> during first-run setup.

Jabberwok supports two inference backends, selected automatically based on model
format:

| Backend               | Format                        | Status      |
| --------------------- | ----------------------------- | ----------- |
| **Parakeet (ONNX)**   | directory (`.tar.gz` extract) | Default     |
| Whisper (whisper.cpp) | single `.bin` file            | Coming soon |

To add a custom model, define a new `[[models.model]]` entry in your config (see
[Configuration](#configuration)) and download it by name:

```sh
jabberwok download-model my-model-name
```

## Configuration

Config file: `~/Library/Application Support/jabberwok/config/jabberwok.toml`

Pass `--config` to any command to use a different file.

### Reference

```toml
[logging]
level = "warn"                        # Global log level (error/warn/info/debug/trace)
targets = ["jabberwok=info"]          # Per-crate overrides

[models]
default = "parakeet-v3"               # Name of the active model

[[models.model]]
name   = "parakeet-v3"
url    = "https://example.com/parakeet-v3-int8.tar.gz"
sha256 = "abc123..."                  # Optional; verified after download
# path is set automatically after download

# Add more [[models.model]] blocks to register additional models

[devices]
# Per-hostname device preferences (use "default" as a fallback key)
[devices.default]
input  = "USB Microphone"            # Device name or index; omit to use OS default
output = "Built-in Output"
```

> **Note:** The recording hotkey (Right Command) is currently hardcoded and not
> yet configurable via the config file.

## Config and Data Paths (macOS)

| Location    | Path                                                            |
| ----------- | --------------------------------------------------------------- |
| Config      | `~/Library/Application Support/jabberwok/config/jabberwok.toml` |
| Models      | `~/Library/Application Support/jabberwok/models/`               |
| Logs        | `~/Library/Logs/jabberwok/`                                     |
| LaunchAgent | `~/Library/LaunchAgents/com.handy.jabberwok.plist`              |

Installed macOS builds bootstrap `config/` into app support, but not `models/`.
Start/stop sounds are embedded into the binary at build time from `media/on.raw`
and `media/off.raw`.

When running inside the installed `.app` bundle with no subcommand:

1. Jabberwok checks whether the default model is installed.
2. Jabberwok checks microphone and accessibility permissions.
3. If anything is missing, it opens the first-run setup window.
4. The daemon starts only after setup is complete.

## CLI Reference

The following subcommands are available from the `jabberwok` binary (or
`cargo run --` when running from source).

### `daemon`

Run as a background service that listens for key events and transcribes speech.
The daemon will not start until the default model is installed and required
permissions are granted.

```sh
jabberwok daemon
jabberwok daemon --model models/my-model.bin
jabberwok daemon --save-utterances
```

### `list-devices`

List all available audio input and output devices.

```sh
jabberwok list-devices
```

### `select-device`

Set the preferred input/output device by index or name. Saved per-hostname in
config.

```sh
jabberwok select-device --input "USB Microphone"
jabberwok select-device --output 2
jabberwok select-device --input "USB" --output "Built-in" --host default
```

Pass `--input ""` or `--input 0` to clear back to the OS default.

### `record`

Record audio from the default input device to a 16 kHz mono WAV file.

```sh
jabberwok record                            # 5s → recording.wav
jabberwok record --duration 10 --output clip.wav
```

### `transcribe`

Transcribe from the microphone or a file.

```sh
jabberwok transcribe                        # record 5s then transcribe
jabberwok transcribe --duration 10
jabberwok transcribe --file clip.wav
jabberwok transcribe --input               # inject result into focused input
jabberwok transcribe --save-utterances     # also save WAV + TXT to utterances/
```

### `download-model`

Download a known model by name and set it as the default. Additional models are
defined in `config/jabberwok.toml`.

```sh
jabberwok download-model parakeet-v3
jabberwok download-model parakeet-v3 --models-dir /tmp/models
```

### `doctor`

Show the same readiness checks used by the macOS first-run setup flow. Exits `0`
when Jabberwok is ready and non-zero when setup is incomplete.

```sh
jabberwok doctor
```

### `permissions`

Open or guide the required permission flows.

```sh
jabberwok permissions microphone
jabberwok permissions accessibility
jabberwok permissions all
```

### `type`

Inject text directly at the currently focused input via the Accessibility API.

```sh
jabberwok type "hello world"
```

## Known Limitations

- **Hotkey is not configurable** — hardcoded to Right Command.
- **macOS only** — the hotkey listener and text injection use macOS-specific
  APIs.
- **No silence detection** — recording ends when you release the key; there is
  no automatic VAD trimming of leading/trailing silence.
- **Text injection may fail in some apps** — sandboxed apps, password fields,
  and apps running with elevated privileges may reject Accessibility-based
  injection.

## Troubleshooting

**Daemon won't start** Run `jabberwok doctor` to see exactly what is missing
(model, permissions, etc.).

**Text isn't injected after transcription**

- Make sure Accessibility permission is granted for the right app (see
  [App vs. terminal](#app-vs-terminal) above).
- Click into a text input before releasing the hotkey.
- Some apps (sandboxed Mac App Store apps, some password managers) block
  Accessibility injection.

**Wrong microphone is being used** Run `jabberwok list-devices` to see available
devices, then `jabberwok select-device --input "Device Name"` to pin one.

**Model download fails** Check your internet connection; the model URL is
defined in your config file and can be overridden if needed.

**Permission shows as granted but still fails after a macOS update** macOS
occasionally resets Accessibility grants after system updates. Re-run
`jabberwok permissions all` and re-grant in System Settings.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[MIT](LICENSE)
