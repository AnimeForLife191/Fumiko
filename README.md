<div align="center">

# Fumiko (文子)
### A private, local-first inbox watcher powered by on-device AI

[![License: MPL 2.0](https://img.shields.io/badge/License-MPL_2.0-blue.svg)](https://opensource.org/licenses/MPL-2.0)
[![Built with Rust](https://img.shields.io/badge/Language-Rust_1.80+-dea584.svg?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![UI: Dioxus](https://img.shields.io/badge/UI-Dioxus_0.7.10-3b82f6.svg)](https://dioxuslabs.com/)
[![Storage: SQLite](https://img.shields.io/badge/Storage-SQLite_WAL-003B57.svg?logo=sqlite&logoColor=white)](https://sqlite.org/)
[![Local AI: Ollama](https://img.shields.io/badge/Local_AI-Ollama-black.svg)](https://ollama.com/)
[![Platform](https://img.shields.io/badge/Platform-Windows_%7C_macOS_%7C_Linux-lightgrey.svg)](#)

[Website](https://animeforlife191.github.io/fumiko.html) • [Documentation](https://animeforlife191.github.io/applications/fumiko/docs.html) • [Privacy Policy](https://animeforlife191.github.io/applications/fumiko/privacy.html) • [Support](https://animeforlife191.github.io/support.html) • [Discussions](https://github.com/AnimeForLife191/Fumiko/discussions)

</div>

---

**Fumiko** is a desktop email watcher that runs completely on your computer.

You give her rules for what you care about (like *"job interviews"*, *"urgent client questions"*, or *"receipts"*), and she keeps an eye on your Gmail and Outlook inboxes in the background. When an email matches your criteria, she flags it using a local AI model running directly on your machine.

**No cloud AI reading your inbox. No tracking. No passwords leaving your machine.**

---

<!-- Optional: Add an application screenshot or demo GIF here -->
<!-- ![Fumiko Dashboard Preview](assets/screenshot-dashboard.png) -->

## What She Does

* **100% Private & Local**: Everything runs on your machine through [Ollama](https://ollama.com). None of your email text, headers, or prompts are ever sent to OpenAI, Google, or any remote server.
* **Smart Two-Tier Scanning (Keeps your PC fast)**: A tiny 1B–3B model quickly checks the subject line and preview first during background sync. Only if an email looks relevant or ambiguous does she load the body for a deeper read, capping memory so your computer doesn't bog down.
* **The Findings Board**: Instead of digging through hundreds of promo emails and newsletters, matched emails get pinned to their own clean dashboard so you don't lose track of what matters.
* **No Passwords Stored**: Connects via OAuth 2.0 with PKCE and loopback ports. Fumiko never sees or saves your actual email password. Refresh tokens are stored directly in your OS credential vault (Windows Credential Manager, Apple Keychain, or Linux Secret Service).
* **Fast & Lightweight**: Built with Rust and backed by SQLite running in WAL mode with UUIDv7 indexing so the inbox loads instantly without lag.
* **Custom Themes**: Don't like the default colors? Drop a simple `custom.css` file into your local app folder to tweak the interface however you like.

---

## Why I Built Fumiko

While looking for work, I found myself constantly stressed out. I was checking my inboxes over and over every day, terrified I’d miss an interview invite or a recruiter reaching out before it got buried under promotional noise. 

On top of that, I was juggling three different inboxes (personal, work, and university). Regular email apps let you put them all in one window, but they don't help you actually sort out what's urgent.

I built Fumiko to take that anxiety off my plate, but also to create an open project others could use and learn from. I'm still early in my journey as a developer, but I put a lot of care into documenting the architecture and keeping the codebase readable so anyone interested in local-first apps or Rust can explore how it works.

---

## Developer Notes & Architecture

Fumiko is organized as a modular Rust workspace. If you're exploring the codebase or want to see how the pieces fit together, I wrote detailed architecture notes for each part:

| Crate / Part | What it handles | Notes |
| :--- | :--- | :--- |
| **`email_core`** | Sync engine, Gmail History API, Microsoft Graph Delta queries, safe cursor updates, and MIME body parsing. | [📖 Sync & Provider Notes](docs/email_core.md) |
| **`oauth`** | RFC 8252 loopback authorization code flow with PKCE, dynamic ephemeral port binding (`127.0.0.1:0`), and constant-time CSRF validation. | [📖 OAuth Architecture](docs/oauth.md) |
| **`local_ai`** | Background Ollama daemon management, streaming model downloads, prompts, and context window limits. | [📖 Local AI Architecture](docs/local_ai.md) |
| **`storage`** | SQLite engine, WAL mode, UUIDv7 indexing, foreign keys, and the OS Keyring `TokenStore` abstraction. | [📖 Storage Architecture](docs/storage.md) |
| **`security`** | Threat model, memory-only access tokens, sanitized logging, and connection safety. | [📖 Security Notes](docs/security.md) |
| **`app`** | Desktop frontend built with Dioxus, reactive state, sanitized iframe rendering, and custom CSS injection. | Source in `/app` |

---

## Getting Started

### Prerequisites

1. **Rust Toolchain**: Install [rustup](https://rustup.rs/) (Rust 1.80+ recommended).
2. **[Dioxus CLI](https://dioxuslabs.com/learn/0.7/getting_started/)**:
   ```bash
   cargo install dioxus-cli
   ```
   *(Check their quickstart guide if your OS needs any specific GUI dependencies).*
3. **Ollama**: Download and install [Ollama](https://ollama.com). Then pull a lightweight model from your terminal (or do it inside the app later):
   ```bash
   ollama pull llama3.2:1b
   ```
   *(You can also use any models you want like `phi4-mini` or `gemma2:2b` if your computer has extra RAM/VRAM).*

### Building and Running from Source

1. Clone the repository:
   ```bash
   git clone https://github.com/AnimeForLife191/Fumiko.git
   cd Fumiko
   ```

2. *(Optional)* Configure development credentials:
   ```bash
   cp .env.example .env
   ```
   *Note: You don't need a `.env` file to run the app. You can type or paste your Google and Microsoft keys straight into the in-app Settings.*

3. Run the desktop app:
   ```bash
   dx serve --platform desktop
   ```

---

## Connecting Your Accounts

Fumiko works out of the box with **personal Microsoft accounts** (`@outlook.com`, `@hotmail.com`, `@live.com`).

> [!NOTE]
> **Account Setup & Verification (Why Gmail needs your own keys)**  
> * **Personal Microsoft Accounts** (`@outlook.com`, `@hotmail.com`): Connect out of the box with zero extra setup.
> * **School & Work Microsoft Accounts**: Will show a **"Need admin approval"** screen because Microsoft requires "Publisher Verification" (a registered company, D-U-N-S number, and Partner Center account). 
> * **Gmail Accounts**: Because Google requires full app verification before allowing public sign-ins for restricted scopes, you'll need to set up your own free Google Cloud keys (it takes ~2 minutes—guide below).
> 
> I'm a 22-year-old solo developer building this project in my spare time, so I can't jump through those enterprise legal hoops on a whim right now. If Fumiko gets enough traction, I'll definitely look into doing the corporate paperwork down the line. Until then, personal Microsoft accounts work seamlessly, and the quick Google setup below gets Gmail running in no time!

To use your own developer credentials, head to **Settings -> OAuth Credentials** inside the app.

For full, step-by-step instructions on creating your own Google and Microsoft keys, see [CREDENTIAL_SETUP.md](CREDENTIAL_SETUP.md).

---

## Custom Theming

Fumiko lets you change the look of the app using custom CSS without having to rebuild the code.

Just make a file named `custom.css` in your local Fumiko data folder:

* **Windows**: `%LOCALAPPDATA%\fumiko\custom.css`
* **macOS**: `~/Library/Application Support/fumiko/custom.css`
* **Linux**: `~/.local/share/fumiko/custom.css`

### Example Palette (`custom.css`)

```css
:root {
    /* 1. Base Surfaces (Pitch Charcoal) */
    --bg-primary: #090b0a;
    --bg-secondary: #101412;
    --bg-tertiary: #161c19;
    --bg-surface: rgba(16, 20, 18, 0.82);
    --bg-surface-raised: rgba(22, 28, 25, 0.90);
    --bg-surface-card: rgba(13, 17, 15, 0.70);
    --bg-surface-translucent: rgba(255, 255, 255, 0.05);
    --bg-hover: rgba(16, 185, 129, 0.12);

    /* 2. Text */
    --text-primary: #f0fdf4;
    --text-secondary: #86efac;
    --text-muted: #52796f;

    /* 3. Primary Accent (Emerald Green) */
    --accent-primary: #10b981;
    --accent-primary-bright: #34d399;
    --accent-primary-deep: #047857;
    --accent-primary-subtle: rgba(16, 185, 129, 0.14);
    --accent-primary-hover: rgba(16, 185, 129, 0.24);
    --accent-primary-glow: rgba(16, 185, 129, 0.38);

    /* 4. Secondary Accent (Electric Cyan) */
    --accent-secondary: #06b6d4;
    --accent-secondary-bright: #22d3ee;
    --accent-secondary-deep: #0e7490;
    --accent-secondary-subtle: rgba(6, 182, 212, 0.14);
    --accent-secondary-hover: rgba(6, 182, 212, 0.24);
    --accent-secondary-glow: rgba(6, 182, 212, 0.38);

    /* 5. Borders */
    --border-subtle: rgba(52, 211, 153, 0.18);
    --border-bright: rgba(52, 211, 153, 0.45);
    --border-accent: rgba(34, 211, 238, 0.45);

    /* 6. Gradients */
    --gradient-primary: linear-gradient(120deg, var(--accent-primary-deep), var(--accent-primary));
    --gradient-accent: linear-gradient(120deg, var(--accent-primary), var(--accent-secondary));
    --gradient-title: linear-gradient(135deg, var(--text-primary), var(--accent-primary-bright));
}
```

---

## Security & Privacy

* **No Secrets in the Database**: Refresh tokens and custom client secrets live in your operating system's native keychain. The local SQLite database only stores basic headers, message metadata, and client ID's.
* **Controlled AI Memory Usage**: Prompts and context lengths are capped (`num_ctx: 2048`, `keep_alive: 1m`) so local models won't eat up all your RAM when classifying in the background.
* **In-Memory Access Tokens**: Short-lived tokens stay in RAM only and are wiped when the app closes.
* **One-Click Local Wipe**: There's a button in Settings that deletes your refresh tokens from the OS keychain, purges the SQLite database, runs a `VACUUM` to clean disk space, and resets the app back to a fresh install.

---

## Part of ShuhariTech

Fumiko is the first tool I'm building under **ShuhariTech**.

The name comes from the martial arts concept of **Shu-Ha-Ri (守破離)**:
1. **Shu (守)**: Learn and respect the fundamentals.
2. **Ha (破)**: Experiment, break convention, and innovate.
3. **Ri (離)**: Transcend the rules and build freely.

That's the philosophy behind this project: learning how things actually work under the hood, being transparent about how it's built, and making tools that respect the user.

---

## Contributing

Found a bug, want to improve the UI, have ideas to make the sync or classification better, or even a new feature you'd like to see? Contributions and feedback of any skill level are welcome!

1. Check open issues or start a thread in [Discussions](https://github.com/AnimeForLife191/Fumiko/discussions).
2. Fork the repo and create a branch (`git checkout -b feature/cool-idea`).
3. Open a Pull Request.

If you're touching sync logic or token handling, please check the checklists in [Security Notes](docs/security.md) and [Provider Notes](docs/email_core.md) first so we don't break cursor invariants.

---

## License

This project is licensed under the [Mozilla Public License 2.0 (MPL-2.0)](LICENSE).