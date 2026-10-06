import http.server


class P(http.server.BaseHTTPRequestHandler):
    def do_CONNECT(self):
        print("PROXY SAW CONNECT", self.path, flush=True)
        self.send_response(502)
        self.end_headers()


http.server.HTTPServer(("127.0.0.1", 8080), P).serve_forever()
