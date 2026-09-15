## First-Time Launch and Installation

Because Fumiko is a free open-source project, binaries are not signed with expensive commercial certificates. On your first launch, Windows and macOS will display standard unrecognized app security prompts. 

Follow these steps to launch the app:

---

### Windows (Microsoft Defender SmartScreen)

1. Extract the downloaded `.zip` archive.
2. Double-click `fumiko.exe`.
3. If Windows displays "Windows protected your PC":
   * Click the underlined text **More info**.
   * Click the **Run anyway** button.
4. Fumiko will open normally. You only need to do this once.

> [!NOTE]
> Make sure to read the [Built-in AI Sidecar](#built-in-ai-sidecar-important-for-windows-and-linux) section below so the `bin/` folder is not separated from `fumiko.exe`.

---

### macOS (Apple Silicon Gatekeeper)

1. Extract `fumiko-aarch64-apple-darwin.tar.gz`.
2. Move `Fumiko.app` into your `/Applications` folder.
3. **First launch only**: Instead of double-clicking:
   * Right-click (or Control-click) on `Fumiko.app` and select **Open**.
   * In the warning prompt ("macOS cannot verify the developer"), click **Open**.

> [!TIP]
> **If macOS displays "Fumiko is damaged and can't be opened":**  
> Apple automatically flags browser downloads with a quarantine attribute. To clear this, open your Terminal and run:
> ```bash
> xattr -cr /Applications/Fumiko.app
> ```
> This removes the web quarantine flag and allows the app to open.

---

### Linux

1. Extract the `.tar.gz` archive:
   ```bash
   tar -xzf fumiko-x86_64-unknown-linux-gnu.tar.gz
   ```
2. Mark the binary as executable:
   ```bash
   chmod +x fumiko
   ```
3. Run the application:
   ```bash
   ./fumiko
   ```

> [!NOTE]
> Make sure to read the [Built-in AI Sidecar](#built-in-ai-sidecar-important-for-windows-and-linux) section below so the `bin/` folder remains next to fumiko.

---

### Built-in AI Sidecar (Important for Windows and Linux)

Fumiko includes a zero-setup local AI engine (`llama-server`) that runs on-device inference without needing Ollama or terminal configuration.

#### Keep the `bin/` folder next to the app
When extracting the Windows `.zip` or Linux `.tar.gz`:
* **Do not move `fumiko.exe` (or `fumiko`) by itself.**
* The **`bin/`** folder must stay directly next to the executable:
  ```text
  Fumiko/
  ├── fumiko.exe (or fumiko)
  └── bin/
      ├── llama-server.exe
      └── *.dll (or *.so)
  ```
*(If you want a shortcut on your Desktop or Start Menu, right-click `fumiko.exe` and select **Create Shortcut** rather than moving the executable itself).*

#### Optional: Standalone AppData Setup
If you prefer running Fumiko as a standalone binary without keeping the `bin/` folder next to it, place the `bin/` folder in your operating system application directory:

* **Windows**: `%LOCALAPPDATA%\fumiko\bin\llama-server.exe`
* **macOS**: Handled automatically inside `Fumiko.app/Contents/Resources/bin/`
* **Linux**: `~/.local/share/fumiko/bin/llama-server` (or anywhere on your system `$PATH`)