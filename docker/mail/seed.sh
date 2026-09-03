#!/bin/sh
# Deliver the bundled .eml fixtures into the test account via Dovecot LDA.
set -eu

MAIL_USER="${MAIL_USER:-dev@mailiner.test}"
LDA="/usr/libexec/dovecot/dovecot-lda"
SEED_DIR="/usr/local/share/mailiner-seed"

# Fixtures are stored with LF in git; LDA wants RFC 5322 CRLF.
crlf_tmp="$(mktemp)"
trap 'rm -f "$crlf_tmp"' EXIT

deliver() {
  mailbox="$1"
  file="$2"
  echo "  seed ${mailbox} <- $(basename "$file")"
  awk '{ printf "%s\r\n", $0 }' "$file" > "$crlf_tmp"
  "$LDA" -d "$MAIL_USER" -m "$mailbox" < "$crlf_tmp"
}

echo "Seeding mailbox for ${MAIL_USER}..."

# Special-use folders exist after the first login/LDA delivery; create them now
# so non-INBOX seeds have a target.
for box in Drafts Sent Trash Junk Archive; do
  doveadm mailbox create -u "$MAIL_USER" "$box" 2>/dev/null || true
done

# INBOX: a representative set of MIME shapes Mailiner has to render.
deliver INBOX "${SEED_DIR}/01-plain-text.eml"
deliver INBOX "${SEED_DIR}/02-html-only.eml"
deliver INBOX "${SEED_DIR}/03-multipart-alternative.eml"
deliver INBOX "${SEED_DIR}/04-mixed-attachment.eml"
deliver INBOX "${SEED_DIR}/05-related-inline-image.eml"
deliver INBOX "${SEED_DIR}/06-html-remote-image.eml"
deliver INBOX "${SEED_DIR}/07-utf8-quoted-printable.eml"
deliver INBOX "${SEED_DIR}/08-base64-plain.eml"
deliver INBOX "${SEED_DIR}/09-thread-root.eml"
deliver INBOX "${SEED_DIR}/10-thread-reply.eml"
deliver INBOX "${SEED_DIR}/11-multiple-attachments.eml"
deliver INBOX "${SEED_DIR}/12-no-subject.eml"
deliver INBOX "${SEED_DIR}/13-html-script.eml"
deliver INBOX "${SEED_DIR}/14-newsletter-nested.eml"

deliver Drafts "${SEED_DIR}/draft-unsent.eml"
deliver Sent "${SEED_DIR}/sent-copy.eml"
deliver Trash "${SEED_DIR}/trashed.eml"

# A few already-read, one flagged, the rest stay unseen.
doveadm flags add -u "$MAIL_USER" '\Seen' mailbox INBOX SUBJECT "Welcome to Mailiner"
doveadm flags add -u "$MAIL_USER" '\Seen' mailbox INBOX SUBJECT "HTML-only announcement"
doveadm flags add -u "$MAIL_USER" '\Seen' mailbox INBOX SUBJECT "Meeting notes"
doveadm flags add -u "$MAIL_USER" '\Seen' mailbox Sent ALL
doveadm flags add -u "$MAIL_USER" '\Seen' mailbox Trash ALL
doveadm flags add -u "$MAIL_USER" '\Flagged' mailbox INBOX SUBJECT "Please review the attached notes"

echo "Seed complete."
doveadm mailbox status -u "$MAIL_USER" "messages unseen" '*'
