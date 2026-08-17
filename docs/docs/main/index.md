---
pageType: home

hero:
  # name: RMK
  text: A feature-rich Rust keyboard firmware
  tagline: Join our Discord server for discussions, support, and community collaboration!
  image:
    src: /logo.svg
    alt: RMK Logo
  actions:
    - theme: brand
      text: Get Started
      link: docs/getting_started/introduction
    - theme: alt
      text: Discord
      link: https://discord.gg/HHGA7pQxkG

features:
  - title: Extensive Microcontroller Support
    details: Powered by embassy, with robust support for STM32, nRF, RP2040, and ESP32
    icon: 🖥️

  - title: Real-time Keymap Configuration
    details: Real-time keymap modification via Vial (enabled by default) or RMK's native Rynk protocol (experimental)
    icon: 🧪

  - title: Advanced Features
    details: Layer switching, media controls and tap-hold keys are available out-of-the-box
    icon: 🕹️

  - title: Wireless Connectivity
    details: BLE wireless support with automatic reconnection and multi-device (tested on nRF52840, ESP32-C3, and ESP32-S3)
    icon: 📡

  - title: Easy Configuration
    details: Define your keyboard through a single keyboard.toml file
    icon: ⚙️

  - title: Performance & Efficiency
    details: Ultra-low 2ms wired/10ms wireless latency with months of battery life
    icon: 🔋
---
