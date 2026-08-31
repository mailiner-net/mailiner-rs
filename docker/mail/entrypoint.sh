#!/bin/sh
# Start Dovecot + Postfix and seed the test mailbox on first run.
set -eu

# Dovecot applies auth_username_format=%Lu before passwd-file lookup.
MAIL_USER="$(printf '%s' "${MAIL_USER:-dev@mailiner.test}" | tr '[:upper:]' '[:lower:]')"
MAIL_PASSWORD="${MAIL_PASSWORD:-dev}"
MAIL_NAME="${MAIL_NAME:-Dev User}"
FORCE_SEED="${FORCE_SEED:-0}"

case "$MAIL_USER" in
  ?*@?*)
    MAIL_LOCAL="${MAIL_USER%@*}"
    MAIL_DOMAIN="${MAIL_USER#*@}"
    ;;
  *)
    echo "MAIL_USER must be an email address (got: $MAIL_USER)" >&2
    exit 1
    ;;
esac

MAIL_HOME="/var/mail/vmail/${MAIL_LOCAL}"
SEEDED_MARKER="${MAIL_HOME}/Maildir/.mailiner-seeded"

mkdir -p /run/dovecot /var/spool/postfix/private /var/spool/postfix/public
chown root:root /run/dovecot

# SASL / LMTP sockets live in the Postfix chroot jail.
mkdir -p /var/spool/postfix/private
chown postfix:postfix /var/spool/postfix/private

HASH="$(doveadm pw -s SHA512-CRYPT -p "$MAIL_PASSWORD")"
{
  echo "${MAIL_USER}:${HASH}:5000:5000:${MAIL_NAME}:${MAIL_HOME}::"
  if [ "$MAIL_LOCAL" != "$MAIL_USER" ]; then
    echo "${MAIL_LOCAL}:${HASH}:5000:5000:${MAIL_NAME}:${MAIL_HOME}::"
  fi
} > /etc/dovecot/users
chmod 600 /etc/dovecot/users

# FORCE_SEED=1 must replace the persisted Maildir, not append another copy.
if [ "$FORCE_SEED" = "1" ]; then
  rm -rf "$MAIL_HOME"
fi
mkdir -p "$MAIL_HOME"
chown -R mailiner:mailiner /var/mail/vmail

# Catch-all: any authenticated SMTP recipient is delivered to MAIL_USER.
printf '/.+/ %s\n' "$MAIL_USER" > /etc/postfix/recipient_canonical
postmap /etc/postfix/recipient_canonical

postconf -e "myhostname = mail.${MAIL_DOMAIN}"
postconf -e "mydomain = ${MAIL_DOMAIN}"
postconf -e "virtual_mailbox_domains = ${MAIL_DOMAIN}"

# Postfix needs Dovecot's auth socket, so start IMAP/LMTP first.
dovecot
if [ ! -f /run/dovecot/master.pid ]; then
  echo "Dovecot failed to start" >&2
  exit 1
fi
# Give imap-login / auth a moment to create sockets.
i=0
while [ "$i" -lt 50 ]; do
  if [ -S /var/spool/postfix/private/auth ] && [ -S /var/spool/postfix/private/dovecot-lmtp ]; then
    break
  fi
  i=$((i + 1))
  sleep 0.1
done
if [ ! -S /var/spool/postfix/private/auth ]; then
  echo "Dovecot auth socket did not appear" >&2
  exit 1
fi
if [ ! -S /var/spool/postfix/private/dovecot-lmtp ]; then
  echo "Dovecot LMTP socket did not appear" >&2
  exit 1
fi

if [ ! -f "$SEEDED_MARKER" ] || [ "$FORCE_SEED" = "1" ]; then
  /usr/local/bin/seed-mail.sh
  su-exec mailiner touch "$SEEDED_MARKER"
fi

postfix start

echo "Mailiner local mail server ready."
echo "  IMAPS  : 993  (implicit TLS)"
echo "  SMTPS  : 465  (implicit TLS)"
echo "  submit : 587  (STARTTLS)"
echo "  user   : ${MAIL_USER}  (also ${MAIL_LOCAL})"
echo "  pass   : (from MAIL_PASSWORD)"

# Keep PID 1 in the foreground; stop both daemons on signal.
term() {
  postfix stop || true
  doveadm stop || true
  exit 0
}
trap term INT TERM

# Dovecot is already daemonized; wait on its master pid, and also
# fail if Postfix dies so PID 1 does not keep a half-working container.
DOVECOT_PID="$(cat /run/dovecot/master.pid)"
while kill -0 "$DOVECOT_PID" 2>/dev/null; do
  if ! postfix status >/dev/null 2>&1; then
    echo "Postfix exited unexpectedly" >&2
    doveadm stop || true
    exit 1
  fi
  sleep 5
done
echo "Dovecot exited unexpectedly" >&2
postfix stop || true
exit 1
