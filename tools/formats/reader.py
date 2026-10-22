"""A binary reader (little-endian, with big-endian and variable-length reads) for prototyping decoders of a game's data files.

    from reader import Reader
    r = Reader(data)
    magic = r.bytes(4); count = r.u32()
    for _ in range(count):
        rid, weight, name = r.u32(), r.f32(), r.pstr8()
    r.done()   # raises if any byte is left: a field you have not found yet

Every read is bounds-checked and says where it failed. `r.block(size)` returns a sub-reader for
a length-prefixed section, so a decoder can check that each section is consumed exactly. Write
the decoder here first, run it over every file of its type with validate.py, and only then
port it to the reimplementation's language.
"""
import struct


class FormatError(Exception):
    pass


class Reader:
    def __init__(self, data, offset=0, end=None, name='data'):
        self.data = data
        self.pos = offset
        self.end = len(data) if end is None else end
        self.name = name

    def _take(self, n):
        if self.pos + n > self.end:
            raise FormatError(f'{self.name}: read of {n} bytes at {self.pos:#x} passes the end ({self.end:#x})')
        out = self.data[self.pos:self.pos + n]
        self.pos += n
        return out

    def _unpack(self, fmt):
        return struct.unpack(fmt, self._take(struct.calcsize(fmt)))[0]

    def u8(self): return self._unpack('<B')
    def i8(self): return self._unpack('<b')
    def u16(self): return self._unpack('<H')
    def i16(self): return self._unpack('<h')
    def u32(self): return self._unpack('<I')
    def i32(self): return self._unpack('<i')
    def u64(self): return self._unpack('<Q')
    def i64(self): return self._unpack('<q')
    def f32(self): return self._unpack('<f')
    def f64(self): return self._unpack('<d')
    # Big-endian numbers: Amiga, Atari ST and Macintosh games, most consoles, network data.
    def u16be(self): return self._unpack('>H')
    def i16be(self): return self._unpack('>h')
    def u32be(self): return self._unpack('>I')
    def i32be(self): return self._unpack('>i')
    def f32be(self): return self._unpack('>f')
    def bool(self): return self.u8() != 0
    def bytes(self, n): return bytes(self._take(n))

    def uleb(self):
        """An unsigned variable-length number, 7 bits per byte, low bits first (LEB128, as in
        protobuf, DWARF, .NET and many save formats). The top bit says another byte follows."""
        n = shift = 0
        while True:
            b = self.u8()
            n |= (b & 0x7F) << shift
            shift += 7
            if not b & 0x80:
                return n

    def sleb(self):
        """A signed LEB128 number: the last byte's bit 6 is the sign."""
        n = shift = 0
        while True:
            b = self.u8()
            n |= (b & 0x7F) << shift
            shift += 7
            if not b & 0x80:
                return n - (1 << shift) if b & 0x40 else n

    def bits(self, n, msb_first=False):
        """The next n bits as a number. Bits are read from the low end of each byte by
        default (LZW and many compressors); msb_first=True reads from the high end (Huffman
        codes in many formats, packed pixels, passwords). Call align() to return to a byte."""
        if not hasattr(self, '_bitbuf') or self._bitorder != msb_first:
            self._bitbuf, self._bitcount, self._bitorder = 0, 0, msb_first
        out = 0
        for i in range(n):
            if self._bitcount == 0:
                self._bitbuf, self._bitcount = self.u8(), 8
            if msb_first:
                bit = (self._bitbuf >> (self._bitcount - 1)) & 1
                out = (out << 1) | bit
            else:
                bit = (self._bitbuf >> (8 - self._bitcount)) & 1
                out |= bit << i
            self._bitcount -= 1
        return out

    def align(self):
        """Drop the bits left in the current byte."""
        self._bitcount = 0

    def magic(self, expected):
        got = self.bytes(len(expected))
        if got != expected:
            raise FormatError(f'{self.name}: magic {got!r} at {self.pos - len(expected):#x}, expected {expected!r}')

    def cstr(self, encoding='latin-1'):
        """A NUL-terminated string."""
        end = self.data.index(b'\0', self.pos, self.end)
        out = self.data[self.pos:end].decode(encoding)
        self.pos = end + 1
        return out

    def pstr8(self, encoding='latin-1'):
        """A string with a one-byte length in front."""
        return self.bytes(self.u8()).decode(encoding)

    def pstr32(self, encoding='latin-1'):
        """A string with a four-byte length in front."""
        return self.bytes(self.u32()).decode(encoding)

    def wstr(self, units):
        """`units` UTF-16 code units."""
        return self.bytes(2 * units).decode('utf-16-le')

    def block(self, size, name=None):
        """A sub-reader over the next `size` bytes; this reader moves past them."""
        start = self.pos
        self._take(size)
        return Reader(self.data, start, start + size, name or f'{self.name}[{start:#x}]')

    def remaining(self):
        return self.end - self.pos

    def done(self):
        if self.pos != self.end:
            raise FormatError(f'{self.name}: {self.end - self.pos} bytes left unread at {self.pos:#x} '
                              f'(next: {self.data[self.pos:self.pos + 16].hex(" ")})')
