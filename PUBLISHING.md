# Yas Mail publisher OAuth setup

This guide is for the person building and distributing Yas Mail. Gmail and Outlook users should only choose their provider and sign in through the browser. They should not register applications or enter client IDs.

The publisher registers Yas Mail once with Google and Microsoft, then supplies the application identities to the native build. The same Google identity serves Gmail and Google Drive; the same Microsoft identity serves Outlook and OneDrive. Mail and drive connections still request their own permissions and save separate authorizations.

The current preview has no publisher Google/Microsoft credentials provisioned. Its unavailable-provider state is intentional. Neither registration, public verification, nor live account sign-in has been completed as part of this implementation.

## Build configuration

Set these variables in the build process before running Cargo/Tauri:

| Variable | Value |
| --- | --- |
| `YAS_MAIL_GOOGLE_CLIENT_ID` | The client ID of Yas Mail's Google OAuth **Desktop app** registration. |
| `YAS_MAIL_GOOGLE_CLIENT_SECRET` | Optional desktop-client metadata if supplied by Google. Use only the Desktop app registration's value. |
| `YAS_MAIL_MICROSOFT_CLIENT_ID` | Yas Mail's Microsoft Entra Application (client) ID, configured as a public desktop application. |

The native app validates configured ID formats before starting authorization. The frontend receives provider names and availability flags; it does not receive client IDs or secrets. A missing or malformed configuration leaves that provider unavailable. These values are compiled into the executable: changing them requires a rebuild, rather than an end-user setting or runtime environment variable.

Client IDs identify the application and are public. Desktop apps cannot keep an embedded secret confidential. This implementation uses authorization-code flow with PKCE; it is not a place to embed confidential web-app credentials or a Microsoft client secret. See [Google's desktop OAuth guidance](https://developers.google.com/identity/protocols/oauth2/native-app).

Set real values in the build environment or CI configuration without committing downloaded credential files. For example, after setting the variables through your chosen build system:

```powershell
cd src-tauri
cargo test --locked
cargo tauri build
```

A build without these variables remains a usable demo with Google/Microsoft sign-in unavailable. A successful build or a `configured` flag confirms local configuration only; it does not prove registration validity, provider approval, or a working live sign-in.

## Google: Gmail and Google Drive

1. Create a Google Cloud project owned by Yas Mail's publisher. Enable Gmail API, Google Calendar API, People API, and Drive API for the features being distributed.
2. Configure Yas Mail's consent-screen branding, audience, contact details, and required scopes. For development, use consented test users as allowed by Google's testing mode.
3. Create an OAuth client with application type **Desktop app**, then set the Google build variables above. Yas Mail opens the system browser and uses `http://127.0.0.1:<random-port>/callback` for its temporary local callback.

Mailbox scopes, each prefixed by `https://www.googleapis.com/auth/`:

- `gmail.modify` for reading, composing, sending, and moving messages.
- `gmail.settings.basic` for sender filters.
- `calendar.events` for calendar events and meetings.
- `contacts` for contacts.

Google Drive separately requests `https://www.googleapis.com/auth/drive.readonly` for the file picker.

Google classifies `gmail.modify` and `gmail.settings.basic` as restricted scopes. Complete the applicable Google verification process before broad public distribution. Testing mode is not evidence that the application is approved for general users. Review the actual data handling and Google's requirements when preparing the production submission; this document does not claim that Yas Mail has passed verification or a security assessment. See [Gmail scope classifications and verification](https://developers.google.com/workspace/gmail/api/auth/scopes).

## Microsoft: Outlook and OneDrive

1. Register Yas Mail in Microsoft Entra under the publisher's directory. Choose an audience that includes **organizational directories and personal Microsoft accounts** to support both Microsoft 365 and Outlook.com. See [Microsoft application registration](https://learn.microsoft.com/en-us/entra/identity-platform/quickstart-register-app).
2. Add the **Mobile and desktop applications** platform with `http://localhost/callback`. Yas Mail chooses a random local port; Microsoft ignores that port when matching localhost redirects while preserving the path. See [Microsoft loopback redirect matching](https://learn.microsoft.com/en-us/entra/identity-platform/reply-url).
3. Set `YAS_MAIL_MICROSOFT_CLIENT_ID` from the registration's **Overview → Application (client) ID**. Use the public desktop flow; no Microsoft client secret is needed.

Outlook requests delegated Microsoft Graph permissions `User.Read`, `Mail.ReadWrite`, `Mail.Send`, `MailboxSettings.ReadWrite`, `Calendars.ReadWrite`, and `Contacts.ReadWrite`, plus `offline_access`.

OneDrive separately requests delegated `User.Read`, `Files.Read`, and `offline_access`. An organization's consent policy can still require administrator approval. A personal-account audience must remain enabled for Outlook.com and personal OneDrive.

## Release verification

Verify a configured build with consented Google and Microsoft test accounts before claiming live support. Check initial sign-in, account identity, token refresh after restart, multiple accounts, cancellation/error recovery, and the mail/calendar/contact/drive features included in the release. Existing synthetic UI and native tests are listed in [VALIDATION.md](VALIDATION.md); they do not replace provider sign-in checks.

Dropbox and Zoho WorkDrive still use manual app details in this preview. Their current setup remains in [CLOUD-DRIVES.md](CLOUD-DRIVES.md). Publisher-managed sign-in for those providers is not implemented in this release.
