"""A minimal ISO 9660 reader for the PS2 discs: list and read files, nothing else.

    python3 tools/ps2/iso.py DISC.iso ls [PATH]
    python3 tools/ps2/iso.py DISC.iso cat PATH > file

Read-only; the disc is never written to.
"""
import struct
import sys

SECTOR = 2048


class Iso:
    def __init__(self, path):
        self.f = open(path, "rb")
        self.f.seek(16 * SECTOR)
        pvd = self.f.read(SECTOR)
        if pvd[1:6] != b"CD001":
            raise SystemExit("not an ISO 9660 image")
        self.root = self._record(pvd[156:190])

    @staticmethod
    def _record(b):
        n = b[0]
        if n == 0:
            return None
        lba = struct.unpack_from("<I", b, 2)[0]
        size = struct.unpack_from("<I", b, 10)[0]
        flags = b[25]
        name = b[33:33 + b[32]].decode("latin-1")
        return {"lba": lba, "size": size, "dir": bool(flags & 2), "name": name, "len": n}

    def read(self, lba, size):
        self.f.seek(lba * SECTOR)
        return self.f.read(size)

    def entries(self, d):
        data = self.read(d["lba"], d["size"])
        out, i = [], 0
        while i < len(data):
            if data[i] == 0:  # the rest of the sector is padding
                i = (i // SECTOR + 1) * SECTOR
                continue
            r = self._record(data[i:i + data[i]])
            i += data[i]
            if r and r["name"] not in ("\x00", "\x01"):
                r["name"] = r["name"].split(";")[0]
                out.append(r)
        return out

    def find(self, path):
        d = self.root
        for part in [p for p in path.split("/") if p]:
            for e in self.entries(d):
                if e["name"].upper() == part.upper():
                    d = e
                    break
            else:
                raise SystemExit("no such path: " + path)
        return d

    def walk(self, d=None, prefix=""):
        d = d or self.root
        for e in self.entries(d):
            yield prefix + e["name"], e
            if e["dir"]:
                yield from self.walk(e, prefix + e["name"] + "/")


def main():
    if len(sys.argv) < 3:
        raise SystemExit(__doc__)
    iso = Iso(sys.argv[1])
    cmd = sys.argv[2]
    if cmd == "ls":
        d = iso.find(sys.argv[3]) if len(sys.argv) > 3 else iso.root
        for e in iso.entries(d):
            print("%10d  %s%s" % (e["size"], e["name"], "/" if e["dir"] else ""))
    elif cmd == "tree":
        for name, e in iso.walk():
            print("%10d  %s" % (e["size"], name))
    elif cmd == "cat":
        e = iso.find(sys.argv[3])
        sys.stdout.buffer.write(iso.read(e["lba"], e["size"]))
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main()
