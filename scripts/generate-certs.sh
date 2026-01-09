#!/bin/bash
#
# Generate TLS certificates for Dendrite HTTPS
#
# This script creates a local Certificate Authority (CA) and server certificate.
# To avoid browser warnings, install the CA certificate on your devices:
#
# - iOS: Email the ca.crt file to yourself, open it, install the profile,
#        then go to Settings > General > About > Certificate Trust Settings
#        and enable full trust for the CA.
#
# - Android: Copy ca.crt to device, go to Settings > Security > Install from storage
#
# - Desktop browsers: Import ca.crt into your browser's certificate store
#   - Chrome: Settings > Privacy > Security > Manage certificates > Authorities > Import
#   - Firefox: Settings > Privacy & Security > Certificates > View Certificates > Authorities > Import
#

set -e

CERT_DIR="${1:-./certs}"
DOMAIN="${2:-dendrite.local}"

# Get all local IP addresses for SAN
get_local_ips() {
    ip addr show | grep "inet " | grep -v "127.0.0.1" | awk '{print $2}' | cut -d'/' -f1
}

echo "=== Dendrite Certificate Generator ==="
echo ""
echo "This will create certificates in: $CERT_DIR"
echo "Domain: $DOMAIN"
echo ""

mkdir -p "$CERT_DIR"
cd "$CERT_DIR"

# Generate CA private key
if [ ! -f ca.key ]; then
    echo "Generating CA private key..."
    openssl genrsa -out ca.key 4096
fi

# Generate CA certificate
if [ ! -f ca.crt ]; then
    echo "Generating CA certificate..."
    openssl req -new -x509 -days 3650 -key ca.key -out ca.crt \
        -subj "/C=US/ST=Local/L=Local/O=Dendrite Local CA/CN=Dendrite Root CA"
fi

# Generate server private key
echo "Generating server private key..."
openssl genrsa -out server.key 2048

# Build SAN (Subject Alternative Name) list
SAN="DNS:$DOMAIN,DNS:localhost"
for ip in $(get_local_ips); do
    SAN="$SAN,IP:$ip"
done
SAN="$SAN,IP:127.0.0.1"

echo "SAN entries: $SAN"

# Create config file for certificate with SAN
cat > server.cnf << EOF
[req]
default_bits = 2048
prompt = no
default_md = sha256
distinguished_name = dn
req_extensions = req_ext

[dn]
C = US
ST = Local
L = Local
O = Dendrite
CN = $DOMAIN

[req_ext]
subjectAltName = $SAN

[v3_ext]
authorityKeyIdentifier=keyid,issuer
basicConstraints=CA:FALSE
keyUsage = digitalSignature, keyEncipherment
extendedKeyUsage = serverAuth
subjectAltName = $SAN
EOF

# Generate server CSR
echo "Generating server certificate signing request..."
openssl req -new -key server.key -out server.csr -config server.cnf

# Sign server certificate with CA
echo "Signing server certificate with CA..."
openssl x509 -req -days 365 -in server.csr -CA ca.crt -CAkey ca.key \
    -CAcreateserial -out server.crt -extensions v3_ext -extfile server.cnf

# Clean up
rm -f server.csr server.cnf ca.srl

echo ""
echo "=== Certificates Generated ==="
echo ""
echo "Files created in $CERT_DIR:"
echo "  ca.crt     - CA certificate (install on devices to trust)"
echo "  ca.key     - CA private key (keep secure!)"
echo "  server.crt - Server certificate"
echo "  server.key - Server private key"
echo ""
echo "To enable HTTPS, add to your dendrite.toml:"
echo ""
echo "[daemon.tls]"
echo "cert = \"$CERT_DIR/server.crt\""
echo "key = \"$CERT_DIR/server.key\""
echo ""
echo "Then install ca.crt on your mobile devices to avoid warnings."
echo ""
