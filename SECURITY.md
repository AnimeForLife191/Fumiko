# Security Policy

Fumiko is built around local-first privacy, and security is a core priority. If you believe you have discovered a vulnerability, please report it responsibly so it can be resolved before public disclosure.

---

## Supported Versions

Security patches and bug fixes are applied to the latest release.

| Version | Supported |
| :--- | :--- |
| Latest release (`v0.1.x` / `main`) | Yes |
| Older releases | No |

---

## How to Report a Vulnerability

Please **do not** open a public GitHub issue for security vulnerabilities.

Instead, please report vulnerabilities using one of these private channels:

1. **GitHub Private Vulnerability Reporting (Preferred)**:
   Navigate to the **Security** tab of this repository, click **Advisories**, and select **Report a vulnerability**. This creates a private draft advisory where we can discuss the issue directly.
2. **Email**:
   Send details directly to `shuharitech@outlook.com`.

### What to Include in Your Report
To help investigate and resolve the issue quickly, please include:
* A clear description of the vulnerability and its potential impact.
* Step-by-step reproduction instructions or a minimal proof of concept.
* Your operating system and the version of Fumiko you were running.

---

## What Happens Next

* **Initial Response**: I will acknowledge your report within 48 to 72 hours.
* **Assessment and Patch**: Once verified, a fix will be developed and tested in a private branch.
* **Release and Disclosure**: A patched release will be published alongside a security advisory crediting you for the discovery (unless you prefer to remain anonymous).

---

## Technical Security Model

If you are looking for technical documentation on how Fumiko handles tokens, OS keyrings, PKCE loopback binding, and HTML sanitization, please see our detailed [Security Architecture and Threat Model](docs/security.md).