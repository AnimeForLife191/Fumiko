# Local AI and Classification Architecture

Local AI in Fumiko is designed around a strict privacy principle: email content should never leave your machine for classification, summarization, or analysis. Rather than sending email text to third-party cloud APIs, Fumiko runs inference directly against local language models hosted on your computer.

This document explains the architecture of the `local_ai` crate, describes dual-backend support (built-in sidecar and external Ollama), details the two-tier classification strategy, and outlines the safeguards used to keep resource usage low.

## Why Local Inference

Routing email through remote language model APIs introduces privacy risks, recurring token costs, and external network dependencies. Local execution provides:

* Complete Privacy: Subject lines, sender addresses, preview snippets, and message bodies are evaluated in memory on the local machine and never transmitted to external cloud servers.
* Offline Processing: Because models run on local hardware, classification and analysis do not rely on remote AI services or an active internet connection.
* Zero Marginal Cost: Users can categorize thousands of emails without paying per-token API fees.
* Zero Barrier to Entry: Non-technical users do not need to install terminal tools or external daemons to run on-device AI.

## Multi-Backend Architecture

The `local_ai` crate uses a modular design centered around the shared `EmailClassifier` trait. Synchronization and UI layers interact with this single trait, allowing the active engine to change without altering mailbox sync logic.

### Core Modules
* `prompt`: Pure, engine-agnostic prompt generation, character-safe truncation, JSON grammar schemas, and markdown code-fence sanitization.
* `builtin`: Plug-and-play sidecar engine powered by a bundled `llama-server` process. Manages direct GGUF model downloads, disk deletions, process lifecycles on port 11435, and `LlamaServerClassifier`.
* `ollama`: Optional external engine for power users running an existing Ollama daemon on port 11434. Manages tag discovery, model pulls, daemon health, and `OllamaClassifier`.
* `lib.rs`: Exposes shared data models (`Criterion`, `Classification`), the common `EmailClassifier` trait, and re-exports from both engines.

## The Built-in Sidecar Engine (`builtin`)

To eliminate setup friction for non-technical users, Fumiko includes a self-contained local inference path that does not require installing Ollama:

### 1. Process Supervision (`LlamaServerService`)
* Dedicated Port: Runs strictly on `http://127.0.0.1:11435` to prevent port collisions with existing Ollama instances running on 11434.
* Headless Execution: Spawns detached standard I/O streams. On Windows, processes launch with `CREATE_NO_WINDOW` (`0x08000000`) so terminal windows never flash on screen. On Linux, companion library paths (`LD_LIBRARY_PATH`) are resolved automatically.
* Health Polling: `wait_until_ready` polls the `/health` endpoint up to 20 times at 500ms intervals before accepting inference requests.

### 2. Standardized Storage Paths
Downloaded GGUF models are stored in standard OS application data folders:
* Windows: `%LOCALAPPDATA%\fumiko\models`
* macOS: `~/Library/Application Support/fumiko/models`
* Linux: `~/.local/share/fumiko/models`

### 3. Streaming Downloads and Atomic Replacement (`ModelDownloader`)
* Models stream directly from Hugging Face via `reqwest`.
* Downloads write to a temporary `.part` file first. Once complete, the file is flushed and renamed atomically to `.gguf`, preventing corrupt partial models if a download is interrupted.
* Progress callbacks stream completion fractions between 0.0 and 1.0 directly to the UI.
* Single-click deletion directly calls `std::fs::remove_file`, giving the user immediate control over reclaimed disk space.

## Two-Tier Classification Strategy

Downloading full email bodies and running deep inference on every incoming message can strain system resources and slow down synchronization. Fumiko evaluates emails in two stages using the active `EmailClassifier`:

### Tier 1: Fast Metadata and Snippet Evaluation
* Method: `classify(subject, sender, snippet, criteria)`
* Input: Email subject, sender address, and the server-provided plain-text snippet preview.
* Characteristics: Near-instant execution running during mailbox sync. Because Gmail, Outlook, and IMAP metadata passes include snippet previews, the active model evaluates the opening lines of every email with zero extra network requests.

### Tier 2: Deep Body Inspection
* Method: `classify_with_body(subject, sender, body, criteria)`
* Input: Subject, sender, and truncated plain-text body.
* Characteristics: Executed only when Tier 1 evaluation is ambiguous (confidence between 0.25 and 0.49) and within a strict per-sync budget. Body content is safely truncated to 4,000 characters using Unicode-safe character boundaries to prevent prompt bloat.

## Resource and Memory Management

Running desktop apps alongside local LLMs requires strict memory budgeting to prevent system RAM exhaustion and disk swap thrashing. The `local_ai` crate enforces several safeguards across both backends:

### 1. Context Window Capping (`num_ctx: 2048`)
By default, modern models allocate KV cache for 8,192 to 128,000 tokens, which can consume 2 GB to 6 GB of RAM regardless of prompt length. We explicitly cap context to 2,048 tokens (`-c 2048` in `llama-server`, `num_ctx: 2048` in Ollama), reducing KV cache allocation to roughly 250 MB while providing ample space for headers and a 4,000-character body.

### 2. Output Token Bounding (128 tokens)
Because classification responses are compact JSON objects of roughly 30 to 50 tokens, limiting output tokens (`n_predict: 128` in `llama-server`, `num_predict: 128` in Ollama) prevents runaway generation loops and caps compute usage per email.

### 3. Single-Slot Execution & Concurrency Pipelining
When spawning inference processes, concurrency is restricted to single-slot execution (`-np 1` in `llama-server`, `OLLAMA_NUM_PARALLEL=1` in Ollama) to prevent allocating duplicate model weights or parallel KV caches in RAM. While `email_core` dispatches up to 4 classification requests concurrently over HTTP, they pipeline cleanly into the server's single evaluation queue with client timeouts, preventing multi-threaded RAM spikes.

### 4. Thread Throttling for UI Liveness
In `llama-server`, the thread count (`-t`) is clamped to half of the available CPU cores (between 1 and 4 threads). This ensures that heavy matrix math does not saturate 100% of CPU cores, keeping the desktop UI smooth and responsive.

### 5. Deterministic Sampling (`temperature: 0.1`)
Inferences run at low temperature to suppress creative variation in favor of reliable, repeatable classification output.

### 6. Memory Release on Shutdown
For the built-in engine, the UI supervisor wraps the child process handle in a global tracker. If the user switches back to Ollama, deletes the active model, or quits the application, the process is terminated and the host operating system instantly reclaims the allocated memory.

## Prompt Engineering and Structured Output Robustness

Small language models (1B to 3B parameters) can be sensitive to formatting drift and prompt whitespace. The `prompt` module applies several defensive layers to guarantee valid outputs:

### 1. Token-Level GBNF Grammar Constraints
Rather than relying solely on string instructions, Fumiko supplies an explicit JSON schema (`classification_json_schema`) describing the required object shape. 

This instructs the sampler to enforce token-level GBNF grammar constraints. The model's sampler masks out disallowed tokens at the logits level, making it physically impossible for the model to emit an unlisted criterion label or syntax error.

### 2. Zero-Indentation Prompts
Multi-line prompt strings in `build_prompt` are left-aligned without leading indentation. Small models are sensitive to prompt whitespace and can mirror arbitrary indentation, causing formatting errors in their responses.

### 3. Markdown Fence Stripping and Brace Extraction
As an extra defensive layer, `clean_json_response` strips leading and trailing markdown code fences and slices the text from the first opening brace `{` to the last closing brace `}`. This discards conversational preamble before passing text to `serde_json`.

### 4. Lenient Deserialization
`ModelClassificationOutput` uses optional fields and default fallbacks:
* `matched`: Defaults to false if omitted.
* `criterion_label`: Parsed as an `Option<String>` to handle null values gracefully.
* `confidence`: Parsed as an `Option<f32>` and clamped between 0.0 and 1.0.

### 5. Hallucination Validation
If a model returns `matched: true` with a criterion label that does not exist in the active user criteria list, the synchronization layer discards the result. The model is never permitted to invent new categories outside user-configured rules.

## Model Catalogs

Fumiko provides verified model catalogs for both engines:

### Built-in Catalog (`BUILTIN_CATALOG`)
Direct links to verified, quantized GGUF weights hosted on Hugging Face:
* Llama 3.2 1B Instruct (Q4_K_M): ~810 MB download. Default recommendation for fast, low-memory classification on standard laptops.
* Qwen 2.5 1.5B Instruct (Q4_K_M): ~1120 MB download. Alternative option with higher reasoning capacity for complex watch rules.

### Ollama Catalog (`PULLABLE_MODELS`)
Curated list of tags for users running external Ollama instances, categorized by target scan tier:
* Fast Tier 1 Metadata Scans: `llama3.2:1b` (~1.3 GB), `llama3.2:3b` (~2.0 GB), `gemma2:2b` (~1.6 GB).
* Deep Tier 2 Reasoning Scans: `phi4-mini` (~3.8 GB), designed for complex full-body evaluation on ambiguous emails.
* Custom External Tags: Users can pull any custom model via terminal; Ollama local tags are automatically discovered and displayed in settings.

## Local AI Checklist

When adding new models, prompt templates, or inference backends, verify these rules:

* Keep All Inferences Local: Never route message content or classification tasks to external cloud services.
* Use 127.0.0.1 for Local Endpoints: Avoid hostname resolution bugs by targeting explicit IPv4 loopback (port 11435 for built-in, port 11434 for Ollama).
* Enforce Schema Constraints: Pass structured JSON schemas to activate grammar-constrained decoding on both backends.
* Bound Context Windows: Always set an explicit context size of 2048 tokens to prevent KV cache memory bloat.
* Cap Output Length: Limit generation to 128 tokens for compact JSON responses.
* Single-Slot Daemon Execution: Constrain daemon concurrency (`-np 1` or `OLLAMA_NUM_PARALLEL=1`) to protect host memory.
* Pass Snippets in Tier 1: Include the server-provided preview snippet to maximize classification accuracy at zero extra network cost.
* Truncate Safely: Use Unicode character boundaries when truncating body text to 4,000 characters.
* Validate Criterion Labels: Reject any model response that attempts to match an unlisted or hallucinated label.
* Clean Up Active Model Settings on Uninstall: Clear the active model setting in storage whenever an installed or downloaded model is deleted.