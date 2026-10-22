#!/usr/bin/env python3
"""Read a network capture (.pcap or .pcapng, from Wireshark or tcpdump) and split it into the
conversations of a game: who talks to whom, on which ports, and the bytes of each message.

Usage: pcap.py CAPTURE                          the flows: endpoints, protocol, packets, bytes
       pcap.py CAPTURE --flow N [--dir a|b]      the messages of flow N, with a hex preview
       pcap.py CAPTURE --flow N --dump DIR       each message as a file (for fields.py), in order
       pcap.py CAPTURE --flow N --stream OUT     a TCP flow's bytes joined into one stream file

A multiplayer game's protocol is the only record of how its clients and server talked. When
the server is gone, a capture made while it ran (yours, or one the community kept) is the
evidence for rebuilding it. Capture on the loopback interface while the game talks to a local
server, or on the network interface for an online game. Then list the flows, pick the game's,
and study its messages with tools/formats/fields.py, which finds the counters, lengths and
types in them.

UDP messages are kept one per datagram. TCP has no message boundaries: --dump writes each
segment that carries data, which is often one message, and --stream joins a direction's
segments in sequence order, for a protocol with its own length fields. Ethernet, Linux
"cooked" and loopback captures are read, with IPv4 and IPv6. Captures are game material when
they hold the game's data: keep them in derived/. Standard library only.
"""
import os
import struct
import sys


def packets(path):
    """(link type, bytes) for each packet, from pcap or pcapng."""
    with open(path, 'rb') as f:
        data = f.read()
    magic = data[:4]
    if magic in (b'\xd4\xc3\xb2\xa1', b'\xa1\xb2\xc3\xd4', b'\x4d\x3c\xb2\xa1', b'\xa1\xb2\x3c\x4d'):
        e = '<' if magic in (b'\xd4\xc3\xb2\xa1', b'\x4d\x3c\xb2\xa1') else '>'
        link = struct.unpack_from(e + 'I', data, 20)[0]
        at = 24
        while at + 16 <= len(data):
            incl = struct.unpack_from(e + 'I', data, at + 8)[0]
            yield link, data[at + 16:at + 16 + incl]
            at += 16 + incl
        return
    if magic == b'\x0a\x0d\x0d\x0a':        # pcapng: blocks
        e = '<' if data[8:12] == b'\x4d\x3c\x2b\x1a' else '>'
        links, at = [], 0
        while at + 12 <= len(data):
            kind, length = struct.unpack_from(e + 'II', data, at)
            if length < 12:
                break
            if kind == 1:                    # interface description
                links.append(struct.unpack_from(e + 'H', data, at + 8)[0])
            elif kind == 6:                  # enhanced packet
                iface, _, _, cap = struct.unpack_from(e + 'IIII', data, at + 8)
                yield links[iface] if iface < len(links) else 1, data[at + 28:at + 28 + cap]
            elif kind == 3:                  # simple packet
                yield links[0] if links else 1, data[at + 16:at + length - 4]
            at += length
        return
    raise SystemExit(f'{path}: not a pcap or pcapng file')


def ip_payload(link, frame):
    """(src, dst, protocol number, payload) from a link-layer frame, or None."""
    if link == 1:                            # Ethernet
        kind, at = struct.unpack_from('>H', frame, 12)[0], 14
        while kind == 0x8100:                # VLAN tags
            kind, at = struct.unpack_from('>H', frame, at + 2)[0], at + 4
    elif link == 113:                        # Linux cooked
        kind, at = struct.unpack_from('>H', frame, 14)[0], 16
    elif link in (0, 108):                   # BSD loopback: a 4-byte family
        fam = struct.unpack_from('<I', frame, 0)[0]
        kind, at = (0x0800 if fam == 2 else 0x86DD), 4
    elif link in (101, 228, 12, 14):         # raw IP
        kind, at = (0x0800 if frame[0] >> 4 == 4 else 0x86DD), 0
    else:
        return None
    if kind == 0x0800:
        ihl = (frame[at] & 0x0F) * 4
        total = struct.unpack_from('>H', frame, at + 2)[0]
        proto = frame[at + 9]
        src = '.'.join(map(str, frame[at + 12:at + 16]))
        dst = '.'.join(map(str, frame[at + 16:at + 20]))
        return src, dst, proto, frame[at + ihl:at + total]
    if kind == 0x86DD:
        proto = frame[at + 6]
        src = ':'.join(f'{x:x}' for x in struct.unpack_from('>8H', frame, at + 8))
        dst = ':'.join(f'{x:x}' for x in struct.unpack_from('>8H', frame, at + 24))
        length = struct.unpack_from('>H', frame, at + 4)[0]
        return src, dst, proto, frame[at + 40:at + 40 + length]
    return None


def messages(path):
    """(flow key, direction 'a' or 'b', proto, seq, bytes) for every packet with data."""
    flows = {}
    for link, frame in packets(path):
        ip = ip_payload(link, frame)
        if not ip:
            continue
        src, dst, proto, body = ip
        if proto == 17 and len(body) >= 8:
            sport, dport = struct.unpack_from('>HH', body, 0)
            payload, seq = body[8:], 0
        elif proto == 6 and len(body) >= 20:
            sport, dport, seq = struct.unpack_from('>HHI', body, 0)
            payload = body[(body[12] >> 4) * 4:]
        else:
            continue
        a, b = (src, sport), (dst, dport)
        key = (min(a, b), max(a, b), 'UDP' if proto == 17 else 'TCP')
        direction = 'a' if a == key[0] else 'b'
        if key not in flows:
            flows[key] = len(flows)
        if payload:
            yield key, direction, key[2], seq, payload


def main(argv):
    if not argv or argv[0] in ('-h', '--help'):
        print(__doc__)
        return 0 if argv else 2
    opt = lambda name, default=None: argv[argv.index(name) + 1] if name in argv else default
    msgs = list(messages(argv[0]))
    order = []
    for key, *_ in msgs:
        if key not in order:
            order.append(key)
    if opt('--flow') is None:
        for i, key in enumerate(order):
            mine = [m for m in msgs if m[0] == key]
            a = sum(len(m[4]) for m in mine if m[1] == 'a')
            b = sum(len(m[4]) for m in mine if m[1] == 'b')
            print(f'{i:3d}  {key[2]}  {key[0][0]}:{key[0][1]} <-> {key[1][0]}:{key[1][1]}  '
                  f'{len(mine)} messages, {a:,} bytes a->b, {b:,} bytes b->a')
        return 0
    key = order[int(opt('--flow'))]
    mine = [m for m in msgs if m[0] == key and (opt('--dir') is None or m[1] == opt('--dir'))]
    if opt('--stream'):
        seen, out = set(), bytearray()
        for m in sorted(mine, key=lambda m: m[3]):
            if (m[3], len(m[4])) not in seen:
                seen.add((m[3], len(m[4])))
                out += m[4]
        with open(opt('--stream'), 'wb') as f:
            f.write(out)
        print(f'{len(out):,} bytes written to {opt("--stream")}')
        return 0
    if opt('--dump'):
        os.makedirs(opt('--dump'), exist_ok=True)
        for i, m in enumerate(mine):
            with open(os.path.join(opt('--dump'), f'{i:05d}-{m[1]}.bin'), 'wb') as f:
                f.write(m[4])
        print(f'{len(mine)} messages written to {opt("--dump")}')
        return 0
    for i, m in enumerate(mine):
        side = f'{key[0][0]}:{key[0][1]} ->' if m[1] == 'a' else f'{key[1][0]}:{key[1][1]} ->'
        print(f'{i:5d} {side:24s} {len(m[4]):6d}  {m[4][:24].hex(" ")}')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
