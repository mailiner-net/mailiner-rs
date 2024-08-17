#!/bin/bash

##################################################
# This script generates a CA certificate and a server certificate for domain test.mailiner.net
# and then signs the server certificate with the CA certificate.
#
# Therefore, add the CA certificate to trusted roots on the client side, so that it will trust
# the server certificate.
#
# The reason this script exists is because we need to pass special instructions to OpenSSL
# in ordere to generate a certificate with a SAN (Subject Alternative Name) field. If we
# only specify the domain in CN, the certificate will not be accepted by Rustls.
#
# The generated certs are committed into git and are valid for 100 years, so it is not necessary
# to run this script again. It can be useful though if we needto generate new certs for
# whatever reason.
#
# Based on https://stackoverflow.com/a/53826340 (CC BY-SA 4.0)
##################################################

SERVER="test.mailiner.net"
CORPORATION=Mailiner.net
CITY=Prague
COUNTRY=CZ

CERT_AUTH_PASS=$(openssl rand -base64 32)
echo "$CERT_AUTH_PASS" > cert_auth_password
CERT_AUTH_PASS=$(cat cert_auth_password)

# create the certificate authority
openssl \
  req \
  -subj "/CN=$SERVER/OU=CA/O=$CORPORATION/L=$CITY/C=$COUNTRY" \
  -new \
  -x509 \
  -passout pass:"$CERT_AUTH_PASS" \
  -keyout ca-cert.key \
  -out ca-cert.crt \
  -days 36500

# create client private key (used to decrypt the cert we get from the CA)
openssl genrsa -out domain.key

# create the CSR(Certitificate Signing Request)
openssl \
  req \
  -new \
  -nodes \
  -subj "/CN=$SERVER/OU=Testing/O=$CORPORATION/L=$CITY/C=$COUNTRY" \
  -sha256 \
  -extensions v3_req \
  -reqexts SAN \
  -key domain.key \
  -out domain.csr \
  -config <(cat /etc/ssl/openssl.cnf <(printf "[SAN]\nsubjectAltName=DNS:%s" "$SERVER")) \
  -days 36500

# sign the certificate with the certificate authority
openssl \
  x509 \
  -req \
  -days 36500 \
  -in domain.csr \
  -CA ca-cert.crt \
  -CAkey ca-cert.key \
  -CAcreateserial \
  -out domain.crt \
  -extfile <(cat /etc/ssl/openssl.cnf <(printf "[SAN]\nsubjectAltName=DNS:%s" "$SERVER")) \
  -extensions SAN \
  -passin pass:"$CERT_AUTH_PASS"

# Prepare private key and certificate for the server
cat domain.key domain.crt > domain-bundle.pem

