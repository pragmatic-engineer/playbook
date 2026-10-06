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
