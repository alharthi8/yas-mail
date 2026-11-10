# Manual mail accounts

Use manual setup when your mail provider or administrator gives you IMAP, POP3, and SMTP server details. No Yas Mail application ID is required for this connection type. Google and Microsoft browser sign-in remain the preferred routes for Gmail, Outlook.com, and Microsoft 365.

## Choose the connection type

| Account type | Receiving | Sending | Main boundary |
| --- | --- | --- | --- |
| IMAP + SMTP | Mail from the IMAP server | SMTP | Recognized standard folders, read/star flags, and server drafts; moves require server support. No contact/calendar synchronization. |
| POP3 + SMTP | Inbox messages through POP3 | SMTP | Messages stay on the server. No remote folders, search, flag changes, deletion, or provider drafts. |
| SMTP only | None | SMTP | A send-only account, without an inbox or received attachments. |
| Microsoft 365 / Exchange Online | Microsoft browser sign-in and Graph | Microsoft browser sign-in and Graph | Requires Yas Mail's publisher registration and the organization's consent policies. |

An on-premises Exchange server can use the IMAP + SMTP or POP3 + SMTP options when its administrator has enabled those services and an authentication method Yas Mail supports. This is mail-protocol access; native EWS, MAPI/HTTP, ActiveSync, Exchange autodiscovery, and Exchange collaboration features are not implemented.

## Connect an account

1. Choose **＋** beside the account tabs, then **Other email / manual setup**.
2. Enter your email address and optional outgoing display name, then select IMAP + SMTP, POP3 + SMTP, or SMTP only. Choose **Server settings →**.
3. For IMAP or POP3, enter the incoming hostname, port, security mode, and username. Use the password or app password your provider supports.
4. Enter the outgoing SMTP hostname, port, and security mode. Keep **Use incoming username and password for SMTP** selected when appropriate, or enter separate SMTP credentials. Clear **My SMTP server requires authentication** only when your administrator explicitly supplies a relay that works that way; the connection still requires TLS.
5. Choose **Test connection**. Yas Mail connects and checks authentication without sending a message or deleting mail. Fix any reported settings before continuing.
6. Choose **Connect account**. The connection is checked again before saving the account. It appears in its own tab with the usual account color, avatar, and signature settings.

Changing a hostname, port, security mode, or credential after a test requires testing the new settings. To change a saved account, open **Settings → Connected accounts → Server settings**, re-enter its passwords, and test before saving. Its account identity, avatar, and signature are preserved.

Passwords are kept in the native Windows Credential Manager when an account is connected, rather than browser preferences or local storage. Closing an unfinished setup discards the form's credential values.

## Common ports

Use the values supplied by your provider; custom ports are supported.

| Service | TLS from connection start | Required STARTTLS upgrade |
| --- | --- | --- |
| IMAP | 993 | 143 |
| POP3 | 995 | 110 |
| SMTP submission | 465 | 587 |

**TLS** protects the connection from its start. **STARTTLS** connects to a protocol port and requires a successful TLS upgrade before credentials are sent. Yas Mail does not offer unencrypted password connections or an option to ignore certificate validation. The hostname must match the server's valid certificate. These common TLS ports are described in [RFC 8314](https://www.rfc-editor.org/rfc/rfc8314.html).

## Microsoft 365 and Exchange

For Microsoft 365 / Exchange Online, choose its browser sign-in option rather than entering a Microsoft password in manual setup. Microsoft has removed Basic authentication for Exchange Online POP and IMAP, including app-password workarounds; an account's SMTP AUTH availability also depends on tenant and mailbox policy. Manual settings do not override those policies. See [Microsoft's authentication guidance](https://learn.microsoft.com/en-us/exchange/clients-and-mobile-in-exchange-online/deprecation-of-basic-authentication-exchange-online) and [SMTP AUTH policy](https://learn.microsoft.com/en-us/exchange/clients-and-mobile-in-exchange-online/authenticated-client-smtp-submission).

For on-premises Exchange, obtain the enabled protocol endpoints, usernames, supported password authentication, and certificate names from your administrator. IMAP/POP3 availability is separate from Outlook desktop connectivity. See [POP3 and IMAP4 in Exchange Server](https://learn.microsoft.com/en-us/exchange/clients/pop3-and-imap4/pop3-and-imap4) and [authenticated SMTP settings](https://learn.microsoft.com/en-us/exchange/clients/pop3-and-imap4/configure-authenticated-smtp).

## Preview limits

Manual accounts handle mail. They do not sync contacts, calendars, meetings, or sender rules, and they do not provide a background service or an offline mailbox cache.

- IMAP reads up to 20 messages per page. Sent, Drafts, Trash, Junk, and Archive need a recognized special-use folder or standard English folder name at the server. Unsupported folders produce an error. Read/unread and star flags operate on the server; archive/trash moves require the server's MOVE support. Yas Mail does not use a broad expunge operation as a fallback.
- IMAP drafts are appended to the server's Drafts folder. After SMTP accepts a message, Yas Mail attempts to append a copy to the IMAP Sent folder. If that copy fails, the message was still submitted: a warning is shown, and the send is not retried automatically. Check Sent/provider delivery before sending again.
- POP3 requires the server's UIDL support and reads up to 20 messages per page. It offers Inbox only, leaves server messages intact, and provides no search, server flag changes, remote deletion, or drafts.
- SMTP-only accounts cannot receive mail. Yas Mail does not add a local Sent archive for POP3 or SMTP-only accounts.
- Outgoing manual mail supports up to 9 MB of file attachments in total and 100 recipients. Received attachment downloads are limited to 5 MB per message. Unusually large protocol responses or messages can produce an explicit size or timeout error.

A successful SMTP submission means the server accepted the message; it does not guarantee final delivery. Never repeat a send only because an IMAP Sent-copy warning appeared.

Live third-party mail servers and Exchange organizations have not been validated as part of the implementation. Completed local protocol and interface checks are recorded in [VALIDATION.md](VALIDATION.md). Use the provider's own server settings and confirm the account's available features before relying on it.
