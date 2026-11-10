#!/usr/bin/env python3
"""Refresh synthetic loopback TLS fixtures. Requires OpenSSL, never changes system trust."""
from pathlib import Path
import subprocess
import tempfile

fixtures = Path(__file__).resolve().parents[1] / "src-tauri/tests/fixtures"
def run(*args):
    subprocess.run(["openssl", *map(str, args)], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)

with tempfile.TemporaryDirectory(prefix="yas-mail-test-certs-") as temporary:
    d = Path(temporary)
    (d / "root.cnf").write_text("""[req]
prompt = no
distinguished_name = dn
x509_extensions = ca
[dn]
CN = Yas Mail synthetic test root
[ca]
basicConstraints = critical,CA:TRUE,pathlen:0
keyUsage = critical,keyCertSign,cRLSign
subjectKeyIdentifier = hash
""")
    (d / "leaf.ext").write_text("""basicConstraints = critical,CA:FALSE
keyUsage = critical,digitalSignature,keyEncipherment
extendedKeyUsage = serverAuth
subjectAltName = DNS:localhost,IP:127.0.0.1
subjectKeyIdentifier = hash
authorityKeyIdentifier = keyid,issuer
""")
    run("req", "-x509", "-newkey", "rsa:2048", "-nodes", "-sha256", "-days", "3650", "-config", d / "root.cnf", "-keyout", d / "root.key", "-out", d / "root.pem")
    run("req", "-new", "-newkey", "rsa:2048", "-nodes", "-subj", "/CN=localhost", "-keyout", d / "leaf.key", "-out", d / "leaf.csr")
    # Apple rejects the old ten-year leaf. Keep this synthetic leaf below 398 days.
    run("x509", "-req", "-in", d / "leaf.csr", "-CA", d / "root.pem", "-CAkey", d / "root.key", "-CAcreateserial", "-days", "365", "-sha256", "-extfile", d / "leaf.ext", "-out", d / "leaf.pem")
    run("x509", "-in", d / "root.pem", "-outform", "DER", "-out", fixtures / "manual-test-root.der")
    run("pkcs12", "-export", "-inkey", d / "leaf.key", "-in", d / "leaf.pem", "-certfile", d / "root.pem", "-passout", "pass:yas-mail-test-fixture", "-out", fixtures / "manual-localhost.p12")
    subprocess.run(["openssl", "x509", "-in", str(d / "leaf.pem"), "-noout", "-dates"], check=True)
print("Refreshed synthetic test fixtures. No certificate was added to system trust.")
