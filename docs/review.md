# mailiner-core review — remaining findings

Five independent reviews of `mailiner-core` (types, connector, storage, submit/errors, body/memory) on 2026-08-26. First-wave cleanups and the bugs that did not need a redesign landed in `6a43500`–`b14f132`. This note is the leftover backlog: still true at those commits, with current file locations.

Severity matches the reviews: **bug** is correctness or user-visible breakage, **suggestion** is a real design gap, **nit** is small.

## Already addressed

Do not re-open these.

| Finding | Landed in |
|---|---|
| Unused `Storage` / `InMemoryStorage`, `AccountMetadata`, `FolderMetadata` | `6a43500` |
| Unused `MessagePart.raw_content`, `LoadedMessage.structure`, `anyhow` | `6a43500` |
| `EmailAddress` concatenated recipients with no separator | `e636015` |
| `load_message` treated a decoded cid image as a successful body | `96ee264` |
| `content_parts()` included hidden cid inlines (`should_prefetch`) | `96ee264` |
| BODYSTRUCTURE cache keyed only by UID (Inbox/Sent collision) | `712a476` |
| `get_envelope` always `SELECT`ed INBOX | `712a476` |
| List index not pruned after our own MOVE/DELETE | `712a476` |
| SMTP errors classified from `Display` text | `ccaef37` |
| Unknown persisted `SendErrorKind` had no `#[serde(other)]` | `ccaef37` |
| Send cancel ignored the oneshot; cancelled outbox marked Failed | `ccaef37` |
| SMTP success reply not truncated | `ccaef37` |
| Attachment download hint used decoded `size` instead of wire size | `b14f132` |
| Send cancel requeued before DATA finished (double-send) | `322db54` |
| Date/sequence list index stale after remote EXPUNGE | `289102a` |
| Envelope had no RFC Message-ID / threading headers | `611fa4d` |
| Stringly `update_envelope_flags` | `5e45729` |

---

## Remaining

### 1. `MessageId` is a folder-scoped IMAP UID in a global newtype

- Severity: suggestion (design)
- Where: [`crates/mailiner-core/src/ids.rs`](../crates/mailiner-core/src/ids.rs), [`crates/mailiner-app/src/message.rs`](../crates/mailiner-app/src/message.rs)
- IMAP UIDs are unique per mailbox. Core `MessageId` is an unvalidated `String` (`MessageId::new(uid.to_string())` in the IMAP connector). The same numeric UID in Inbox and Sent is the same id. `::new` also accepts empty strings.
- The app cannot use the core type as-is: it has a second `MessageId` (and `MailboxId`) and round-trips with `to_string()`.
- Adding `folder_id` to `get_envelope` (`712a476`) closed the immediate footgun; every other message API already took a folder. The identity model is still leaky.
- Direction: make `MessageId` folder-scoped (or require `FolderId` on every message op), share one ID type with the app (`From`/`AsRef` without string copies), reject empty IDs.

### 2. List index and Date/sequence paging after a remote EXPUNGE

- [x] Done. Date pages by `UID SEARCH ALL` (newest UID first). Each range `SELECT`s and rebuilds the index when `EXISTS` ≠ cached total. Sequence fetch remains only if SEARCH ALL fails.
- Still open (not required to close this item): IDLE / periodic NOOP so the UI notices EXPUNGE without a later range fetch.

### 3. Envelope has no RFC 5322 identity / threading headers

- [x] Done. `Envelope` now has `reply_to`, `rfc_message_id`, `in_reply_to`, and `references`. IMAP parses them from headers; reply/reply-all prefill sets draft threading and prefers `Reply-To`.

### 4. Flags are stringly typed; star vs flag is unused

- [x] Done. `EnvelopeFlag` is the trait argument; IMAP maps it to atoms. `Starred` (`\Starred`) and `Flagged` (`\Flagged`) stay distinct; the app still does not surface either.

### 5. Unused `EmailConnector` methods

- [x] Done. Removed `create_folder`, `delete_folder`, `open_folder`, `list_envelopes`, and unused `get_envelope` from the trait and both impls.

### 6. `MockConnector` does not implement the contracts

- [x] Done. Documented as a loader/UI fixture. Arrival list is newest-first, HTML is 7bit to match the bytes, `move_messages` returns no dest UIDs, `supports_size_sender` is false. Still not a stateful IMAP double (`sync_unread_sort_index` is empty).

### 7. Prefetch has no size bound; viewer holds wire + decoded + data-URL copies

- [x] Done (budget). Prefetch skips parts whose `original_size` is over 2 MiB. Still open: drop `MessageContent::Binary` after cid inlining (`inlined_part_ids` is unused for that).

### 8. `fetch_raw_parts` swallows missing sections

- [x] Done. A missing requested section is now an error; the loader already fails if no display part decoded.

### 9. MOVE fallback can copy without deleting

- [x] Done. COPY-then-delete failure is `MailinerError::PartialMove`. The UI keeps the source rows and tells the user not to retry.

### 10. `folder_counts` treats missing `UNSEEN` as zero

- [x] Done. STATUS without `UNSEEN` is skipped (same as a failed STATUS); the sidebar keeps its last badge instead of showing a false zero.

### 11. Send cancel can still double-send

- [x] Done. Cancel keeps the in-flight slot (`Sending`) until `SmtpFinished`. Success deletes the row even if `inflight` was already dropped; `Cancelled` is what requeues.

### 12. `MailinerError` is stringly typed; connect classification re-parses English

- [x] Done. `Auth` and `Tls` variants; IMAP login/TLS map into them. `classify_mailiner_error` matches those first. Other `Connector(String)` still uses a substring fallback. Send path stays on `SendErrorKind`.

### 13. `EmailConnector` is parameterized on the stream type

- Severity: suggestion
- Where: [`crates/mailiner-core/src/connector.rs`](../crates/mailiner-core/src/connector.rs)
- The whole trait is generic on `S` plus `async_trait`’s default `Send` futures. The live app stores a concrete `ImapConnector<WebSocketStream>`, so this is not what blocks WASM today, but `load_message` still needs a dummy `NullStream`, and you cannot hold a transport-erased connector after `connect`. `S` is only consumed by `connect`.
- Direction: associated type, or split `connect(stream)` from a non-generic session trait used by list/fetch/stream.

### 14. `MessageSort::Date` is arrival, not the Date header

- [x] Done. Variant is `Arrival` (label “Arrival”). Persisted `"date"` still loads; serde has `alias = "date"`.

### 15. `PartKind::Other` is never produced

- [x] Done. Removed. Unknown leaves stay `Attachment`; the formatter only treats `TextPlain` as plain text.

### 16. Leftover `Account` / timestamps / `TlsModeUnsupported`

- [x] `TlsModeUnsupported` removed. Persisted `"tls_mode_unsupported"` becomes `Permanent` via `#[serde(other)]`.
- [x] `authenticate` returns `Result<()>`. Unused core `Account` type and write-only `created_at` / `updated_at` on `Folder` / `Envelope` dropped. Serde still ignores leftover fields in existing mail-cache blobs. App-level account store / `DraftDocument` timestamps kept.

---

## Suggested order

1. Folder-scoped `MessageId` (1) — unblock a lot of later IMAP work; do not mix with a feature PR.
3. Slim the connector trait (13).
