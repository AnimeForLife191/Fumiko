# Custom OAuth Credential Setup Guide

Fumiko connects directly to Google and Microsoft using standard OAuth 2.0 with PKCE. Because Fumiko is an independent open-source application without enterprise corporate verification, supplying your own developer credentials ensures:

* **Zero Middlemen**: You communicate directly with Google and Microsoft API servers.
* **No Cloud Costs**: Free personal developer tiers provide more than enough quota for personal email sync.
* **100% Data Control**: You control the OAuth client configuration and permissions.

Setup takes about **2 to 3 minutes** per provider. Follow the step-by-step guides below.

---

## 1. Google (Gmail)

Google classifies mailbox reading (`gmail.readonly`) as a restricted scope. Creating your own free Google Cloud project allows you to generate personal client keys with zero restrictions.

### Step-by-Step Setup

1. Open the **[Google Cloud Console](https://console.cloud.google.com)** and create a new project (e.g., `Fumiko Email`).
2. Go to the **[API Library](https://console.cloud.google.com/apis/library)**, search for **Gmail API**, and click **Enable**.
   *(Note: The API must be enabled first, or the required scopes will not appear in later steps).*
3. Under the **Google Auth Platform** menu:
   * Navigate to **[Audience](https://console.cloud.google.com/auth/audience)**, set User Type to **External**, and add your personal email address under **Test users**.
   * Navigate to **[Data Access / Scopes](https://console.cloud.google.com/auth/scopes)**, click **Add or Remove Scopes**, filter for the Gmail API, and select `.../auth/gmail.readonly`.
4. Go to **[Credentials](https://console.cloud.google.com/apis/credentials)**:
   * Click **Create Credentials → OAuth client ID**.
   * Set Application type to **Desktop App**.
   * Name it (e.g., `Fumiko Desktop`) and click **Create**.
5. Click **Download JSON** (or copy your **Client ID** and **Client Secret**).
6. In Fumiko:
   * Navigate to **Settings → OAuth Credentials**.
   * Under Google, click **Import credentials.json** (or manually paste your Client ID and Secret).
   * Click **Save Credentials**.

---

### Important Notes for Google

> [!NOTE]
> **The 7-Day Token Expiration (Testing Mode)**  
> When a Google Cloud project has a publishing status of **Testing**, Google automatically expires refresh tokens after **7 days**. This means you will need to re-authorize your Gmail account once a week.
> 
> **How to get permanent tokens (Pro Tip):**  
> Under **Google Auth Platform → Audience**, click **Publish App** to switch the status from **Testing** to **In Production**. You do *not* need to complete the verification audit for personal use.  
> 
> When you log in with an unverified production app, Google will display a one-time safety screen saying *"Google hasn't verified this app"*. Simply click **Advanced → Go to Fumiko (unsafe)** to complete the login. Your refresh tokens will no longer expire every 7 days.

---

## 2. Microsoft (Outlook / Microsoft 365)

Microsoft uses a public native client registration. You only need an **Application (client) ID**; no client secret is required.

### Step-by-Step Setup

1. Open the **[Azure Portal](https://portal.azure.com)** and navigate to **[App registrations](https://portal.azure.com/#view/Microsoft_AAD_RegisteredApps/ApplicationsListBlade)**.
2. Click **New registration**:
   * **Name**: Enter `Fumiko` (or any preferred name).
   * **Supported account types**: Select **Accounts in any organizational directory (Any Microsoft Entra ID tenant - Multitenant) and personal Microsoft accounts**.
3. Under **Redirect URI (optional)** on the same page:
   * Select platform: **Public client/native (mobile & desktop)**.
   * Enter URI: `http://127.0.0.1/`
4. Click **Register**.
5. Navigate to **API permissions**:
   * Click **Add a permission → Microsoft Graph → Delegated permissions**.
   * Verify or add the following three permissions:
     * `Mail.Read`
     * `User.Read` *(mandatory for personal accounts to retrieve your profile/address)*
     * `offline_access` *(mandatory to receive refresh tokens)*
6. Return to the **Overview** blade, copy your **Application (client) ID**, and paste it into Fumiko under **Settings → OAuth Credentials**.

---

### Important Notes for Microsoft

> [!NOTE]
> **Personal vs. School/Work Accounts**  
> * **Personal Accounts (`@outlook.com`, `@hotmail.com`, `@live.com`)**: Connect seamlessly without any extra approval.
> * **School and Work (Office 365 Enterprise) Accounts**: Microsoft restricts unverified multi-tenant apps from corporate/institutional tenants. If you are linking an institutional account, your organization's IT department may need to approve the client ID before you can sign in.

> [!TIP]
> **Do NOT enable "Allow public client flows"**  
> You may see a toggle in Azure under Authentication labeled *"Allow public client flows"*. Keep this **disabled (No)**. Fumiko uses the browser-based authorization code flow with PKCE via loopback, which does not require legacy non-interactive flows.

---

## 3. How Fumiko Stores Your Credentials

Fumiko enforces strict credential isolation:

* **Client IDs**: Stored locally in your indexed SQLite settings table.
* **Client Secrets & Refresh Tokens**: Saved directly into your operating system's native credential vault:
  * **Windows**: Windows Credential Manager
  * **macOS**: Apple Keychain
  * **Linux**: FreeDesktop Secret Service API / Keyutils
* **Memory-Only Access Tokens**: Temporary access tokens remain strictly in RAM with proactive 50-minute expirations and are never written to disk or logs.

---

## Troubleshooting

### "This app isn't verified" (Google)
This is normal for personal Google Cloud projects. Click **Advanced**, then click **Go to Fumiko (unsafe)**. Since you own the Google Cloud project, you are simply authorizing your own application to read your own mailbox.

### "Need admin approval" (Microsoft)
You are attempting to log into a corporate or educational Microsoft 365 tenant. Your organization's tenant policy blocks unverified applications. You can link personal Microsoft accounts without issue, or ask your IT administrator to grant consent to your registered Application ID.

### Loopback Timeout / Port Stuck
Fumiko binds to dynamic port `127.0.0.1:0` to eliminate port collisions. If you close your browser before completing the sign-in, the request will automatically time out after 60 seconds, or you can click **Cancel Connecting** in the app to immediately release the listener.