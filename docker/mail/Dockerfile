# Local IMAP (Dovecot) + SMTP (Postfix) for Mailiner development.
# Not for production. Ships a test-only CA and a seeded mailbox.
FROM alpine:3.21

RUN apk add --no-cache \
        dovecot \
        dovecot-lmtpd \
        postfix \
        openssl \
        su-exec \
        ca-certificates \
    && addgroup -g 5000 mailiner \
    && adduser -D -u 5000 -G mailiner -h /var/mail/vmail -s /sbin/nologin mailiner \
    && mkdir -p /var/mail/vmail /etc/ssl/mail /etc/dovecot \
    && chown -R mailiner:mailiner /var/mail/vmail

COPY dovecot.conf /etc/dovecot/dovecot.conf
COPY postfix/main.cf /etc/postfix/main.cf
COPY postfix/master.cf /etc/postfix/master.cf
COPY tls/ca.crt tls/server.crt tls/server-chain.crt /etc/ssl/mail/
COPY tls/server.key /etc/ssl/mail/server.key
COPY seed /usr/local/share/mailiner-seed
COPY seed.sh /usr/local/bin/seed-mail.sh
COPY entrypoint.sh /usr/local/bin/entrypoint.sh

RUN chmod 644 /etc/ssl/mail/ca.crt /etc/ssl/mail/server.crt /etc/ssl/mail/server-chain.crt \
    && chmod 600 /etc/ssl/mail/server.key \
    && chmod 755 /usr/local/bin/entrypoint.sh /usr/local/bin/seed-mail.sh \
    && newaliases

EXPOSE 25 143 465 587 993

HEALTHCHECK --interval=15s --timeout=5s --start-period=20s --retries=6 \
    CMD postfix status && doveadm service status >/dev/null

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
