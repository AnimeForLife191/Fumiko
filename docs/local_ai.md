# Local AI and Classification Architecture

Local AI in Fumiko is designed around a strict privacy principle: user email content should never leave the local machine for classification, summarization, or analysis. Rather than sending email text to third-party cloud APIs, Fumiko runs inference directly against local models running on the user hardware.

This document explains the architecture of the `local_ai` crate, describes our two-tier classification strategy, details how we manage local inference backends and streaming model pulls, and outlines the invariants we follow to ensure predictable outputs from small language models.

## Why Local Inference

Routing user emails through remote language model APIs introduces significant privacy risks, token costs, and network dependencies. Local execution gives us:

* Complete Privacy: Subject lines, sender addresses, snippet previews, and message bodies are evaluated in memory on the local machine and never transmitted to external servers.
* Offline Capability: Inbox categorization continues working even when the user is offline or traveling with intermittent internet connectivity.
* Zero Marginal Cost: Users can categorize thousands of historical emails without incurring per-token API charges.

## Multi-Backend Architecture and Extensibility

While the initial implementation uses Ollama as its primary inference engine, the `local_ai` crate is structured to support multiple local AI runtimes over time.

### Current Layout

* `ollama`: Contains the classifier, prompt templates, and direct API interaction with Ollama's generation endpoint.
* `service`: Manages background daemon processes, health checks, model verification, and pull streams.
* `models_list`: Manages the curated catalog of recommended models, local tag discovery, and memory tier classification.

### Future Backend Strategy

Future backends (such as embedded llama.cpp bindings, ONNX runtimes, or custom sidecar processes) will share the common `Criterion` and `Classification` data types. The goal is to allow the UI and synchronization layers to request classifications through a unified interface regardless of whether the active model is managed via an external daemon or an embedded library.

## Two-Tier Classification Strategy

Running deep classification on every incoming email using large models can strain system resources and slow down synchronization. To balance accuracy with performance, Fumiko separates classification into two distinct tiers:

### Tier 1: Fast Metadata and Snippet Classification

* Method: `classify(subject, sender, snippet, criteria)`
* Target Hardware: Lightweight 1B to 3B parameter models (such as Llama 3.2 1B or Gemma 2 2B).
* Input: Email subject, sender address, and the server-provided plain-text snippet preview.
* Characteristics: Near-instant execution running concurrently during mailbox sync. Because Gmail and Outlook provide snippet previews in their lightweight metadata responses, the model receives the opening sentences of every email for zero extra network cost.

### Tier 2: Deep Body Inspection

* Method: `classify_with_body(subject, sender, body, criteria)`
* Target Hardware: Higher-capacity reasoning models (such as Phi-4 Mini or 3B+ models).
* Input: Subject, sender, and truncated body text.
* Characteristics: Executed only when header classification is ambiguous or when specific criteria require full message context. Body content is strictly truncated to a character budget (4000 characters) to prevent large reply chains or HTML boilerplate from ballooning the prompt context.

## Prompt Engineering and JSON Output Robustness

Small language models (1B to 3B parameters) are prone to formatting drift, markdown wrapping, and occasional hallucinations. The `local_ai` crate applies several defensive layers to guarantee valid outputs:

### 1. Zero-Indentation Prompts

Multi-line prompt strings in `build_prompt` are left-aligned without leading indentation. Small models are sensitive to prompt whitespace and can mirror arbitrary indentation, causing formatting corruptions in their responses.

### 2. Lenient Deserialization

Ollama is instructed to return structured JSON (`format: "json"`). However, smaller models frequently omit properties when no match is found, or return `null` instead of empty strings.

`ModelClassificationOutput` uses optional fields and default fallbacks:

* `matched`: Defaults to false if omitted.
* `criterion_label`: Parsed as an `Option<String>` to handle nulls gracefully.
* `confidence`: Parsed as an `Option<f32>` and clamped between 0.0 and 1.0.

### 3. Markdown Fence Stripping

Even in JSON mode, models occasionally wrap their JSON output inside markdown code blocks (such as ````json ... ````). Before passing text to `serde_json`, the classifier strips leading and trailing markdown fences.

### 4. Hallucination Validation

If a model returns `matched: true` with a criterion label that does not exist in the active user criteria list, the synchronization layer discards the result. The model is never permitted to invent new categories outside user-configured rules.

## Local Daemon Management and Health Checks

The `OllamaService` handles the operational lifecycle of the local inference daemon:

### IPv4 Loopback Routing

All network calls to local AI services use `http://127.0.0.1:11434` rather than `localhost`. This prevents operating systems from resolving `localhost` to IPv6 `::1`, which can cause connection refused errors if the daemon is listening strictly on an IPv4 socket.

### Background Spawning and Readiness Polling

When a user enables AI features:

1. `is_running()` sends a lightweight GET request with a short 2-second timeout to check if the server is already active.
2. If inactive, `serve()` spawns `ollama serve` as a background process with detached standard I/O streams.
3. The service polls the health endpoint up to 10 times at 500ms intervals before returning a success outcome or surfacing an actionable error.

## Model Catalog and Tag Normalization

The crate maintains a curated list of verified models (`PULLABLE_MODELS`) categorized by resource footprint:

* Tier 1 Models: Optimized for speed and low RAM usage on standard laptops.
* Tier 2 Models: Optimized for reasoning quality where extra memory is available.

Catalog strings use `Cow<'static, str>` so that catalog definitions can remain `const` while allowing full Serde serialization and deserialization without lifetime constraints.

### Tag Alias Matching

Ollama automatically appends `:latest` to untagged models upon download (for example, pulling `phi4-mini` results in a local tag named `phi4-mini:latest`).

The catalog matching logic normalizes these names using `names_match`, ensuring that installed models with or without explicit `:latest` suffixes correctly map to catalog entries without creating duplicate items in the user interface.

## Streaming Pulls and Progress Handling

Model downloads can range from 1 GB to over 4 GB. `pull_model` consumes the server streaming HTTP response and reports progress in real time.

### Newline-Delimited JSON Buffering

Ollama streams progress updates as newline-delimited JSON chunks. Because TCP packet boundaries do not always align with line breaks, the service buffers incoming bytes and extracts only complete, newline-terminated JSON lines.

### Stream Error Detection

If a download fails midway (such as out-of-disk-space or network loss), Ollama transmits an error object inside the progress stream rather than closing the connection with an HTTP error status. 

`PullProgress` includes an optional `error` field. When an error payload is detected in the stream, the pull loop terminates immediately and returns the provider error message rather than failing with a confusing JSON parsing error.

## Local AI Checklist

When adding new models, prompt templates, or inference backends, verify these invariants:

* Keep All Inferences Local: Never route message content or classification tasks to external cloud services.
* Use 127.0.0.1 for Local Endpoints: Avoid hostname resolution bugs by targeting explicit IPv4 loopback.
* Pass Snippets in Tier 1: Include the server-provided preview snippet to maximize classification accuracy at zero extra network cost.
* Bound Prompt Content: Truncate body inputs to a predictable character limit to protect context windows and memory.
* Strip Markdown Fences: Clean model output strings before passing them to JSON parsers.
* Use Lenient JSON Structs: Ensure all classification output fields have default fallbacks for missing keys.
* Validate Criterion Labels: Reject any model response that attempts to match an unlisted or hallucinated label.
* Normalize Model Tags: Account for implicit `:latest` tag suffixes when comparing local models against catalog lists.
* Handle Stream Errors Explicitly: Check for server error fields inside progress streams during model downloads.