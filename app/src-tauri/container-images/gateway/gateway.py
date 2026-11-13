"""Transparent gateway of one Claude-Hydra container account.

The workspace container shares this container's network namespace. Every TCP connection is
redirected (redsocks) into an HTTP CONNECT to the account's Claude-Hydra relay on the Mac;
DNS is answered by DNS-over-HTTPS through the same relay. Nothing else may leave: no UDP, no
IPv6, no direct route. If the relay is down or paused by the exit guard, the workspace has no
network at all.

The relay address is read from /etc/hydra-gateway/upstream ("ip:port", mounted read-only) at
start, so Hydra can move the relay to another port with a gateway restart.
"""
import ipaddress
import os
import signal
import socket
import socketserver
import struct
import subprocess
import threading
import time
import urllib.request

with open('/etc/hydra-gateway/upstream') as f:
    host, _, port = f.read().strip().rpartition(':')
proxy_ip = str(ipaddress.IPv4Address(host))
proxy_port = int(port)
if not 0 < proxy_port < 65536:
    raise SystemExit('invalid upstream port')
proxy_port = str(proxy_port)

def rule(*args):
    subprocess.run(args, check=True)

# Default deny is installed before any listener or workload starts. Never clear it.
for command in ('iptables', 'ip6tables'):
    for chain in ('INPUT', 'OUTPUT', 'FORWARD'):
        rule(command, '-w', '-P', chain, 'DROP')
    rule(command, '-w', '-F')
rule('iptables', '-w', '-A', 'OUTPUT', '-o', 'lo', '-j', 'ACCEPT')
# OUTPUT REDIRECT can retain the original output-interface metadata.
rule('iptables', '-w', '-A', 'OUTPUT', '-d', '127.0.0.1', '-j', 'ACCEPT')
rule('iptables', '-w', '-A', 'INPUT', '-i', 'lo', '-j', 'ACCEPT')
rule('iptables', '-w', '-A', 'OUTPUT', '-d', proxy_ip, '-p', 'tcp', '--dport', proxy_port, '-j', 'ACCEPT')
rule('iptables', '-w', '-A', 'INPUT', '-s', proxy_ip, '-p', 'tcp', '--sport', proxy_port,
     '-m', 'conntrack', '--ctstate', 'ESTABLISHED', '-j', 'ACCEPT')
rule('iptables', '-w', '-t', 'nat', '-F', 'OUTPUT')
rule('iptables', '-w', '-t', 'nat', '-A', 'OUTPUT', '-d', proxy_ip, '-p', 'tcp', '--dport', proxy_port, '-j', 'RETURN')
rule('iptables', '-w', '-t', 'nat', '-A', 'OUTPUT', '-d', '127.0.0.0/8', '-j', 'RETURN')
for protocol in ('udp', 'tcp'):
    rule('iptables', '-w', '-t', 'nat', '-A', 'OUTPUT', '-p', protocol, '--dport', '53',
         '-j', 'REDIRECT', '--to-ports', '53')
rule('iptables', '-w', '-t', 'nat', '-A', 'OUTPUT', '-p', 'tcp', '-j', 'REDIRECT', '--to-ports', '12345')

config = '''base { log_debug = off; log_info = on; log = stderr;
 daemon = off; redirector = iptables; }
redsocks { local_ip = 127.0.0.1; local_port = 12345;
 ip = %s; port = %s; type = http-connect; }
''' % (proxy_ip, proxy_port)
with open('/run/redsocks.conf', 'w') as f:
    f.write(config)
relay = subprocess.Popen(['redsocks', '-c', '/run/redsocks.conf'])

proxy = urllib.request.ProxyHandler({'https': 'http://%s:%s' % (proxy_ip, proxy_port)})
opener = urllib.request.build_opener(proxy)
limit = threading.BoundedSemaphore(32)

def resolve(query):
    if len(query) < 12 or len(query) > 65535:
        return b''
    if not limit.acquire(blocking=False):
        return b''
    try:
        request = urllib.request.Request('https://cloudflare-dns.com/dns-query', data=query,
            headers={'Content-Type': 'application/dns-message', 'Accept': 'application/dns-message'})
        with opener.open(request, timeout=8) as response:
            result = response.read(65536)
        if len(result) < 12 or len(result) > 65535:
            raise ValueError('Invalid DNS response')
        return query[:2] + result[2:]
    except Exception:
        # SERVFAIL, never fall back to local/system/public UDP DNS.
        flags = struct.unpack('!H', query[2:4])[0]
        return query[:2] + struct.pack('!H', (flags & 0x7900) | 0x8082) + query[4:6] + b'\0' * 6 + query[12:]
    finally:
        limit.release()

class UDP(socketserver.BaseRequestHandler):
    def handle(self):
        data, sock = self.request
        result = resolve(data)
        if result:
            sock.sendto(result, self.client_address)

class TCP(socketserver.BaseRequestHandler):
    def handle(self):
        self.request.settimeout(10)
        def read(size):
            result = b''
            while len(result) < size:
                chunk = self.request.recv(size-len(result))
                if not chunk:
                    raise EOFError()
                result += chunk
            return result
        try:
            size = struct.unpack('!H', read(2))[0]
            result = resolve(read(size))
            if result:
                self.request.sendall(struct.pack('!H', len(result)) + result)
        except (OSError, EOFError):
            pass

class UDPServer(socketserver.ThreadingUDPServer):
    daemon_threads = True
class TCPServer(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True

servers = [UDPServer(('127.0.0.1', 53), UDP), TCPServer(('127.0.0.1', 53), TCP)]
for server in servers:
    threading.Thread(target=server.serve_forever, daemon=True).start()

def shutdown(*_):
    if os.path.exists('/run/gateway-ready'):
        os.unlink('/run/gateway-ready')
    relay.terminate()
    raise SystemExit(0)
signal.signal(signal.SIGTERM, shutdown)
signal.signal(signal.SIGINT, shutdown)

# Relay must be listening before Hydra attaches or enters the workspace.
for attempt in range(50):
    try:
        with socket.create_connection(('127.0.0.1', 12345), timeout=1):
            break
    except OSError:
        if relay.poll() is not None or attempt == 49:
            raise
        time.sleep(0.1)
with open('/run/gateway-ready', 'w') as f:
    # Hydra compares this line with the relay it expects before entering.
    f.write('upstream=%s:%s\n' % (proxy_ip, proxy_port))
print('Gateway ready: only %s:%s can leave this network namespace.' % (proxy_ip, proxy_port), flush=True)
status = relay.wait()
if os.path.exists('/run/gateway-ready'):
    os.unlink('/run/gateway-ready')
raise SystemExit(status or 1)
