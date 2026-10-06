#!/bin/sh
set -eu
S=/private/tmp/claude-501/-Users-isantos-Workspace-pragmatic-engineer-playbook/e1a6b893-0596-41b4-9cfd-b7771e8b4180/scratchpad/ca
rm -rf "$S"; mkdir -p "$S"; cd "$S"
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -days 30 \
  -subj "/CN=Spike Test CA" -keyout ca.key -out ca.pem \
  -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign,cRLSign"
openssl req -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -subj "/CN=localhost" \
  -keyout leaf.key -out leaf.csr
printf "subjectAltName=DNS:localhost,DNS:host.containers.internal,IP:127.0.0.1\nextendedKeyUsage=serverAuth\nbasicConstraints=CA:FALSE\n" > ext.cnf
openssl x509 -req -in leaf.csr -CA ca.pem -CAkey ca.key -CAcreateserial -days 30 -extfile ext.cnf -out leaf.pem
cat > srv.py <<'EOF'
import http.server, ssl
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        b = b'{"tag_name":"v9.9.9-local"}'
        self.send_response(200); self.send_header("Content-Length", str(len(b))); self.end_headers(); self.wfile.write(b)
    def do_CONNECT(self):
        self.send_response(200); self.end_headers()
ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
ctx.load_cert_chain("leaf.pem", "leaf.key")
s = http.server.HTTPServer(("0.0.0.0", 8443), H)
s.socket = ctx.wrap_socket(s.socket, server_side=True)
s.serve_forever()
EOF
ls
