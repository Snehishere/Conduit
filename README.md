# Conduit

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![GitHub Release](https://img.shields.io/github/v/release/Snehishere/Conduit?style=flat)](https://github.com/Snehishere/Conduit/releases)
[![GitHub Stars](https://img.shields.io/github/stars/Snehishere/Conduit?style=social)](https://github.com/Snehishere/Conduit/stargazers)

**Seamless device synchronization with privacy at its core.**

Conduit is a cross-platform application that keeps your devices in sync — clipboard, files, SMS, calls, and more — with end-to-end encryption. No cloud services, no data collection, just direct device-to-device communication.

---

## Features

- 📋 **Clipboard Sync** — Copy on one device, paste on another instantly
- 📁 **File Transfer** — Share files between devices with drag-and-drop
- 💬 **SMS Forwarding** — Send and receive text messages from your computer
- 📞 **Call Handling** — Answer, reject, or dial calls from any device
- 🖥️ **Screen Mirroring** — View your mobile screen on desktop
- 🎮 **Remote Control** — Control your phone from your computer
- 🔒 **End-to-End Encryption** — All data encrypted before transmission
- 🚫 **No Cloud** — Direct peer-to-peer communication
- 🌐 **Cross-Platform** — Windows, macOS, Linux, Android, iOS

---

## Screenshots

*Coming soon*

---

## Download

### Desktop
- **Windows**: [Download .exe](https://github.com/Snehishere/Conduit/releases)
- **macOS**: [Download .dmg](https://github.com/Snehishere/Conduit/releases)
- **Linux**: [Download .AppImage](https://github.com/Snehishere/Conduit/releases)

### Mobile
- **Android**: [Download APK](https://github.com/Snehishere/Conduit/releases) | F-Droid *(coming soon)*
- **iOS**: TestFlight *(coming soon)*

---

## Getting Started

### Prerequisites
- Desktop: Windows 10+, macOS 10.15+, or Linux
- Mobile: Android 8.0+ or iOS 13.0+

### Installation

1. Download the appropriate version for your platform
2. Install the app on all devices you want to sync
3. Open Conduit on both devices
4. Follow the pairing wizard to connect devices
5. Start syncing!

### First-Time Setup

1. Launch Conduit on your primary device
2. Tap "Add Device" or "Pair New Device"
3. A QR code will appear on one device
4. Scan the QR code with your other device
5. Devices will connect automatically

---

## Building from Source

### Desktop (Tauri + React + Rust)

**Prerequisites:**
- Node.js 18+
- Rust 1.70+
- Platform-specific dependencies (see [Tauri Prerequisites](https://tauri.app/v1/guides/getting-started/prerequisites))

**Build:**
```bash
cd desktop
npm install
npm run tauri build
```

### Mobile (Flutter)

**Prerequisites:**
- Flutter 3.0+
- Android SDK (for Android builds)
- Xcode (for iOS builds)

**Build:**
```bash
cd mobile
flutter pub get
flutter build apk  # Android
flutter build ios  # iOS
```

---

## Architecture

### Tech Stack
- **Desktop**: Tauri (Rust + React + TypeScript)
- **Mobile**: Flutter (Dart)
- **Communication**: WebSocket with custom protocol
- **Encryption**: End-to-end AES-256-GCM

### How It Works

1. **Device Discovery**: Local network broadcast or manual pairing
2. **Secure Pairing**: QR code exchange + cryptographic handshake
3. **Encrypted Tunnel**: All messages encrypted with device-specific keys
4. **Direct Communication**: No relay servers, pure P2P

---

## Roadmap

- [x] Core sync functionality (clipboard, files)
- [x] Mobile SMS and call forwarding
- [x] Screen mirroring and remote control
- [ ] Multi-device group sync
- [ ] Browser extension
- [ ] End-to-end encrypted chat
- [ ] Automation rules (IFTTT-style)
- [ ] Multi-language support

See the [DEVELOPMENT_PLAN.md](./DEVELOPMENT_PLAN.md) for detailed progress.

---

## Contributing

Contributions are welcome! See [CONTRIBUTING.md](./CONTRIBUTING.md) for guidelines.

### Ways to Contribute
- 🐛 Report bugs via [GitHub Issues](https://github.com/Snehishere/Conduit/issues)
- 💡 Suggest features via [GitHub Discussions](https://github.com/Snehishere/Conduit/discussions)
- 🔧 Submit pull requests
- 📖 Improve documentation
- 🌍 Add translations

---

## Security

Conduit takes privacy seriously:
- **End-to-end encryption** for all data in transit
- **No cloud storage** — everything is peer-to-peer
- **No analytics** — we don't track you
- **Open source** — audit the code yourself

Found a security vulnerability? Please report it via [GitHub Security Advisories](https://github.com/Snehishere/Conduit/security/advisories/new).

---

## License

This project is licensed under the MIT License - see the [LICENSE](./LICENSE) file for details.

---

## Acknowledgments

Built with:
- [Tauri](https://tauri.app/) — Desktop framework
- [Flutter](https://flutter.dev/) — Mobile framework
- [React](https://react.dev/) — UI library
- [Rust](https://www.rust-lang.org/) — System programming language

---

## Support

- 📧 Email: **150748998+Snehishere@users.noreply.github.com**
- 💬 Discussions: [GitHub Discussions](https://github.com/Snehishere/Conduit/discussions)
- 🐛 Issues: [GitHub Issues](https://github.com/Snehishere/Conduit/issues)

---

**Made with ❤️ by [Sneh Patel](https://github.com/Snehishere)**
