# Cloud drives in Yas Mail

Connect Google Drive, OneDrive, Dropbox, or Zoho WorkDrive under **Settings → Cloud drives**. These connections are separate from email accounts; a Google or Microsoft email connection does not automatically grant drive access. More than one drive account can be connected.

For **Google Drive and OneDrive**, choose the provider and continue in your browser. There are no application-ID or secret fields. These providers use the Google/Microsoft application identities supplied once by Yas Mail's publisher. Drive authorization remains separate from mailbox authorization, even when both use the same publisher application.

The current preview has no publisher Google/Microsoft credentials provisioned. Google Drive and OneDrive therefore show that sign-in is unavailable in this build. The publisher must configure and rebuild Yas Mail using [PUBLISHING.md](PUBLISHING.md); end users do not need their own registrations.

**Dropbox and Zoho WorkDrive still use manual application setup in this preview.** Their current requirements are below. Sign-in and consent take place in your normal browser. Yas Mail stores credentials and refresh tokens in Windows Credential Manager; the browser interface does not save client credentials in local storage.

## Dropbox and Zoho preview setup

| Provider | Application and read permissions | Callback |
| --- | --- | --- |
| Dropbox | Create a scoped **Full Dropbox** application to browse the whole drive. App Folder registrations only see their app folder. Enable `account_info.read`, `files.metadata.read`, `files.content.read`, `sharing.read`. Use the app key; PKCE requires no app secret. | Register the exact `http://localhost:53682/callback`. |
| Zoho WorkDrive | Register a mobile/desktop OAuth client in the account’s data centre. Provide its client ID and client secret. Requested scopes: `WorkDrive.users.READ`, `WorkDrive.team.READ`, `WorkDrive.teamfolders.READ`, `WorkDrive.files.READ`, `ZohoSearch.securesearch.READ`. | Register the exact `http://localhost:53683/callback`. |

Zoho supports United States, Europe, India, Australia, Japan, Canada, China, United Arab Emirates, and Saudi Arabia data centres. Select the region of the Zoho account. The app uses the documented accounts/API/download hosts for that region. Registering in a different region can cause consent or token errors.

The fixed Dropbox and Zoho callback ports must be free while connecting. Google and Microsoft use a temporary random loopback port handled by Yas Mail. Provider consent policies or an organization's administrator can restrict access.

Official setup references: [Google desktop OAuth](https://developers.google.com/identity/protocols/oauth2/native-app), [Google Drive scopes](https://developers.google.com/identity/protocols/oauth2/scopes), [Microsoft public-client OAuth](https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-auth-code-flow), [Microsoft loopback matching](https://learn.microsoft.com/en-us/entra/identity-platform/reply-url), [Dropbox OAuth](https://docs.dropboxapi.com/dropbox-api/docs/oauth), [Dropbox offline access and PKCE](https://dropbox.tech/developers/using-oauth-2-0-with-offline-access), [Zoho desktop authorization](https://www.zoho.com/developer/oauth/mobile-and-desktop-apps/get-authorization-code.html), [Zoho desktop token requirements](https://www.zoho.com/developer/oauth/mobile-and-desktop-apps/get-access-token.html), and [Zoho regional support](https://www.zoho.com/workdrive/developer/docs/api/v1/getting-started-multi-dc-support.html).

## Use files in an email

Open a message draft and choose **Attach from cloud**. The picker stays inside the compose dialog, keeping the sender, recipients, subject, body, and signature intact. Select a connected drive, browse folders, search, and load further results.

- **Attach copy** downloads that file into the current draft. Connect an email account first. Local and cloud files share the existing total attachment allowance: 9 MB for Google email and 3 MB for Outlook. Existing attachments are preserved when you choose more files; use **Remove** beside an attachment to free space.
- **Insert link** retrieves an existing provider URL and inserts plain text into the message. Yas Mail does not create a public link or change permissions. Recipients may need access granted separately at the provider. Dropbox must already have a direct shared link; otherwise the picker reports that no existing link is available.
- Google-native Docs, Sheets, and Slides and Zoho-native Writer, Sheet, and Show files are link-only in this preview. Ordinary uploaded files can be attached when the provider permits downloads and the size fits the email limit.

Sending and saving a draft wait until selected file/link operations finish. Closing the picker keeps the message intact. If the compose dialog is closed or replaced during an operation, its result is not added to another message. The user must still click **Send message**; selecting a cloud file never sends mail automatically.

## Scope and limits

This is cloud attachment/link integration, not Windows folder sync. The connectors do not upload, delete, move, edit, or change cloud sharing permissions. OneDrive browses the current user’s drive; a separate SharePoint library picker is not included. Zoho exposes available teams, team folders, My Folders, and their contents according to provider access.

Disconnecting removes the saved connection from Yas Mail. It leaves provider files and existing permissions in place. Revoke the application at the provider separately if you want to remove its granted access there.

No live provider accounts were authorized or tested during implementation. See [VALIDATION.md](VALIDATION.md) for completed checks and remaining gaps. Live Google/Microsoft workflows require publisher-configured builds and consented accounts; Dropbox/Zoho additionally retain the manual setup described above.
