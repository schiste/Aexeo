import json
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from unittest.mock import patch
from urllib.error import HTTPError

import review


class LocalServer:
    def __init__(self, handler):
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    def __enter__(self):
        self.thread.start()
        return f"http://127.0.0.1:{self.server.server_port}"

    def __exit__(self, *args):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)


class HttpReviewTests(unittest.TestCase):
    def test_local_http_request_uses_provider_schema(self):
        seen = []
        class Handler(BaseHTTPRequestHandler):
            def do_POST(self):
                body = self.rfile.read(int(self.headers["Content-Length"]))
                seen.append((self.path, self.headers.get("Authorization"), json.loads(body)))
                data = json.dumps({"model": "jev-pinned", "answers": {"support": {
                    "type": "choice", "choice": "supported", "probabilities": {"supported": 0.95}}}}).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.end_headers()
                self.wfile.write(data)
            def log_message(self, *args):
                pass
        with LocalServer(Handler) as endpoint, patch.object(review, "ENDPOINT", endpoint + "/v1/systemone"):
            result = review.fetch_review("claim", "source", "jev-pinned", "uncertain", 0.9, "fake-key")
        path, authorization, body = seen[0]
        self.assertEqual(path, "/v1/systemone")
        self.assertEqual(authorization, "Bearer fake-key")
        self.assertEqual(body["state"], {"claim": "claim", "source": "source"})
        self.assertEqual(result["support"], "supported")
        self.assertEqual(result["reliability"], "uncertain")

    def test_redirect_is_rejected_without_forwarding_key(self):
        destination_requests = []
        class Destination(BaseHTTPRequestHandler):
            def do_POST(self):
                destination_requests.append(self.headers.get("Authorization"))
                self.send_response(200)
                self.end_headers()
            def log_message(self, *args):
                pass
        with LocalServer(Destination) as destination:
            class Redirect(BaseHTTPRequestHandler):
                def do_POST(self):
                    self.send_response(307)
                    self.send_header("Location", destination + "/capture")
                    self.end_headers()
                def log_message(self, *args):
                    pass
            with LocalServer(Redirect) as endpoint, patch.object(review, "ENDPOINT", endpoint + "/v1/systemone"):
                with self.assertRaises(HTTPError) as redirect:
                    review.fetch_review("claim", "source", "jev-pinned", "uncertain", 0.9, "fake-key")
                redirect.exception.close()
        self.assertEqual(destination_requests, [])


if __name__ == "__main__":
    unittest.main()
