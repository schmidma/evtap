# evtap

> Deep analysis for your typing mechanics.

`evtap` is a tool that runs in the background, listening to your global keyboard input to analyze how you type, not just what you type.

Unlike web-based typing tests, evtap measures your real-world usage: coding, writing emails, or chatting, to help you identify physical bottlenecks, hesitation patterns, and error-prone key transitions.

## 🚀 Features

evtap goes beyond WPM. It breaks down your typing into three core dimensions:

1. Rhythm & Flow (The "Feel")

- Flight Time (Hesitation): Measures the time taken to move from releasing one key to pressing the next. Identifies keys you struggle to find or reach.
- Bigram Speed (Flow): Tracks your speed on common two-letter sequences (e.g., th, er, in). Essential for optimizing "rolling" layouts.
- Dwell Time: Measures how long you hold a key down. High dwell times often indicate uncertainty or awkward hand positioning.

2. Accuracy & Corrections

- Mistake Analysis: Tracks backspace usage to identify which keys you delete most often.
- Confusion Matrix: Identifies specific substitution patterns (e.g., "I keep typing o when I mean p").

3. Usage Statistics

- Heatmap: Visualizes your most used keys.
- Total Presses: Lifetime tracking of your keyboard usage.

## 🛠 Prerequisites

evtap currently supports Linux only.

It relies on the Linux kernel's evdev subsystem.

### Dependencies

TODO

### Permissions

To listen to global input events, your user must have permission to read from `/dev/input/`.

Option 1 (Recommended): Add user to input group

```
sudo usermod -a -G input $USER
```

You must log out and log back in for this to take effect!

Option 2: Run as root

```
sudo evtap
```

## 📦 Installation

From Crates.io

TODO

From Source

```
git clone [https://github.com/yourusername/evtap.git](https://github.com/yourusername/evtap.git)
cd evtap
cargo run --release
```

## 🔒 Privacy & Security

evtap is technically a keylogger. It has to be, in order to function. Be aware of that.

## 🤝 Contributing

Contributions are welcome! Whether it's a new metric, better UI visualization, or any other thing, feel free to open a PR.
