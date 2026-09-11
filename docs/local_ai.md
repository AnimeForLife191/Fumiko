# Local AI and Classification Architecture

Local AI in Fumiko is designed around a strict privacy principle: email content should never leave your machine for classification, summarization, or analysis. Rather than sending email text to third-party cloud APIs, Fumiko runs inference directly against local language models hosted on your computer.

This document explains the architecture of the `local_ai` crate, describes our two-tier classification strategy, details how we manage local inference daemons and streaming model downloads, and outlines the safeguards we use to keep memory usage low on small models.

---

## Why Local Inference

Routing email through remote language model APIs introduces privacy risks, ongoing token costs, and external network dependencies. Local execution gives us:

* **Complete Privacy**: Subject lines, sender addresses, preview snippets, and message bodies are evaluated in memory on the local machine and never transmitted to external cloud servers.
* **Offline Processing**: Because models run on local hardware, classification and analysis do not rely on remote AI services or an active internet connection.
* **Zero Marginal Cost**: Users can categorize thousands of emails without paying per-token API fees.

---

## Multi-Backend Architecture and Extensibility

While the initial implementation uses Ollama as its primary engine, the `local_ai` crate is structured to support multiple local AI runtimes over time.

### Current Modules
* `ollama`: Contains the classifier, prompt templates, inference parameters, and direct HTTP interaction with Ollama's generation endpoint.
* `service`: Manages background daemon processes, environment isolation, health checks, model verification, and streaming downloads.
* `models_list`: Manages the curated catalog of recommended models, local tag discovery, and memory tier classification.

### Future Runtime Strategy
Future backends (such as embedded llama.cpp bindings, ONNX runtimes, or custom sidecar processes) will share the common `Criterion` and `Classification` data types. The UI and synchronization layers can request classifications through a unified interface regardless of whether the active model is managed via an external daemon or an embedded library.

---

## Two-Tier Classification Strategy

Downloading full email bodies and running deep inference on every incoming message can strain system resources and slow down synchronization. To balance speed with accuracy, Fumiko evaluates emails in two stages using your selected local model:

### Tier 1: Fast Metadata and Snippet Evaluation
* **Method**: `classify(subject, sender, snippet, criteria)`
* **Input**: Email subject, sender address, and the server-provided plain-text snippet preview.
* **Characteristics**: Near-instant execution running during mailbox sync. Because Gmail and Outlook include snippet previews in their lightweight metadata responses, the active model evaluates the opening lines of every email with zero extra network requests.

### Tier 2: Deep Body Inspection
* **Method**: `classify_with_body(subject, sender, body, criteria)`
* **Input**: Subject, sender, and truncated plain-text body.
* **Characteristics**: Executed only when Tier 1 evaluation is ambiguous (confidence between 0.25 and 0.49) and within a strict per-sync budget. Body content is safely truncated to 4,000 characters using Unicode-safe character boundaries to prevent prompt bloat.

---

## Resource and Memory Management

Running desktop apps alongside local LLMs requires strict memory budgeting to prevent system RAM exhaustion and disk swap thrashing. The `local_ai` crate enforces several safeguards:

### 1. Context Window Capping (`num_ctx: 2048`)
By default, modern models allocate KV cache for 8,192 to 128,000 tokens, which can consume 2 GB to 6 GB of RAM regardless of prompt length. We explicitly cap `num_ctx` to 2,048 tokens, reducing KV cache allocation to roughly 250 MB while providing ample space for headers and a 4,000-character body.

### 2. Output Token Bounding (`num_predict: 128`)
Because classification responses are compact JSON objects of roughly 30 to 50 tokens, setting `num_predict` prevents runaway generation loops and caps compute usage per email.

### 3. Idle Memory Release (`keep_alive: "1m"`)
Ollama's default 5-minute keep-alive keeps gigabytes of model weights pinned in RAM long after sync operations finish. We configure a 1-minute timeout so the host operating system recovers RAM quickly between periodic sync batches.

### 4. Single-Slot Execution (`OLLAMA_NUM_PARALLEL=1`)
When spawning the Ollama daemon, environment variables (`OLLAMA_NUM_PARALLEL=1`, `OLLAMA_MAX_LOADED_MODELS=1`) prevent Ollama from allocating duplicate model slots or parallel KV caches in RAM.

### 5. Dedicated Queue-Safe HTTP Timeout (90 seconds)
Because `OLLAMA_NUM_PARALLEL=1` processes inferences one at a time, dispatching multiple concurrent requests causes Ollama to queue them internally. To prevent requests from failing with client-side timeout errors while waiting in line, `OllamaClassifier` uses an explicit 90-second HTTP timeout.

---

## Prompt Engineering and Structured Output Robustness

Small language models (1B to 3B parameters) are prone to formatting drift, markdown wrapping, and occasional hallucinations. The `local_ai` crate applies several defensive layers to guarantee valid outputs:

### 1. JSON Schema Grammar Constraints
Rather than passing a generic string like `format: "json"`, Fumiko supplies an explicit JSON schema (`serde_json::Value`) describing the required object shape. 

This instructs the underlying llama.cpp sampler to enforce GBNF grammar constraints at the token-generation level. The model is constrained by the sampler and physically cannot emit tokens that violate the schema, preventing omitted fields and syntax corruptions.

### 2. Zero-Indentation Prompts
Multi-line prompt strings in `build_prompt` are left-aligned without leading indentation. Small models are sensitive to prompt whitespace and can mirror arbitrary indentation, causing formatting errors in their responses.

### 3. Low Temperature Sampling (`temperature: 0.1`)
We use a near-zero temperature to suppress creative sampling in favor of deterministic classification and consistent JSON syntax.

### 4. Markdown Fence Stripping and Brace Extraction
As an extra defensive layer, `clean_json_response` strips leading and trailing markdown code fences and slices the text from the first opening brace `{` to the last closing brace `}`. This discards conversational preamble before passing text to `serde_json`.

### 5. Lenient Deserialization
`ModelClassificationOutput` uses optional fields and default fallbacks:
* `matched`: Defaults to false if omitted.
* `criterion_label`: Parsed as an `Option<String>` to handle null values gracefully.
* `confidence`: Parsed as an `Option<f32>` and clamped between 0.0 and 1.0.

### 6. Hallucination Validation
If a model returns `matched: true` with a criterion label that does not exist in the active user criteria list, the synchronization layer discards the result. The model is never permitted to invent new categories outside user-configured rules.

---

## Local Daemon Management and Health Checks

The `OllamaService` handles the operational lifecycle of the local inference daemon:

### IPv4 Loopback Routing
All network calls to local AI services use `http://127.0.0.1:11434` rather than `localhost`. This prevents operating systems from resolving `localhost` to IPv6 `::1`, which can cause connection refused errors if the daemon is listening strictly on an IPv4 socket.

### Background Spawning and Readiness Polling
When a user enables AI features:
1. `is_running()` sends a lightweight GET request with a short 2-second timeout to check if the server is already active.
2. If inactive, `serve()` spawns `ollama serve` with `OLLAMA_NUM_PARALLEL=1` and `OLLAMA_MAX_LOADED_MODELS=1` as a background process with detached standard I/O streams.
3. On Windows, the process is spawned with the `CREATE_NO_WINDOW` flag (`0x08000000`) so an intrusive black console window does not appear on the user's desktop.
4. The service polls the health endpoint up to 10 times at 500ms intervals before returning a success outcome or surfacing an actionable error.

---

## Model Catalog and Hardware Tiers

The crate maintains a curated list of verified models (`PULLABLE_MODELS`) categorized by computer resource footprint:
* **Tier 1 Catalog Models**: Smallest footprints (1B to 3B parameters) optimized for speed and low RAM usage on standard laptops (such as Llama 3.2 1B/3B and Gemma 2 2B).
* **Tier 2 Catalog Models**: Higher capacity models (3B+ parameters) for computers with extra RAM or dedicated GPUs (such as Phi-4 Mini).
* **Custom and External Models**: You are not limited to the curated list. Because Fumiko queries Ollama's local tags directly, any model you install via your terminal (such as Qwen 2.5, Mistral, or custom fine-tunes) will automatically appear in settings and can be selected as your active classifier.

### Tag Alias Matching
Ollama automatically appends `:latest` to untagged models upon download (for example, pulling `phi4-mini` results in a local tag named `phi4-mini:latest`).

The catalog matching logic normalizes these names using `names_match`, ensuring that installed models with or without explicit `:latest` suffixes map to catalog entries without creating duplicate items in the interface.

---

## Streaming Pulls and Progress Handling

Model downloads can range from 1 GB to over 4 GB. `pull_model` consumes the server streaming HTTP response and reports progress in real time.

### Newline-Delimited JSON Buffering
Ollama streams progress updates as newline-delimited JSON chunks. Because TCP packet boundaries do not always align with line breaks, the service buffers incoming bytes and extracts only complete, newline-terminated JSON lines.

### Stream Error Detection
If a download fails midway (such as running out of disk space or losing network connectivity), Ollama transmits an error object inside the progress stream rather than closing the connection with an HTTP error status. 

`PullProgress` includes an optional `error` field. When an error payload is detected in the stream, the pull loop terminates immediately and returns the provider error message rather than failing with a confusing JSON parsing error.

---

## Local AI Checklist

When adding new models, prompt templates, or inference backends, verify these rules:

* Keep All Inferences Local: Never route message content or classification tasks to external cloud services.
* Use 127.0.0.1 for Local Endpoints: Avoid hostname resolution bugs by targeting explicit IPv4 loopback.
* Enforce Schema Constraints: Pass a structured JSON schema in the `format` field to activate grammar-constrained decoding.
* Configure Safe Client Timeouts: Maintain a generous HTTP timeout (such as 90 seconds) on the classifier client to accommodate single-slot request queues.
* Bound Context Windows (`num_ctx`): Always set an explicit context size (such as 2048) to prevent KV cache memory bloat.
* Cap Output Length (`num_predict`): Limit generation to the minimum required for structured JSON responses.
* Release Idle Memory (`keep_alive`): Configure a short keep-alive duration so model weights do not permanently lock system RAM.
* Single-Slot Daemon Execution: Constrain daemon concurrency (`OLLAMA_NUM_PARALLEL=1`) to protect host memory.
* Pass Snippets in Tier 1: Include the server-provided preview snippet to maximize classification accuracy at zero extra network cost.
* Truncate Safely: Use Unicode character boundaries when truncating body text to 4,000 characters.
* Validate Criterion Labels: Reject any model response that attempts to match an unlisted or hallucinated label.
* Normalize Model Tags: Account for implicit `:latest` tag suffixes when comparing local models against catalog lists.
* Handle Stream Errors Explicitly: Check for server error fields inside progress streams during model downloads.