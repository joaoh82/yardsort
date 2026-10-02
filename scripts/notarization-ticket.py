#!/usr/bin/env python3
"""Say who signed a Mach-O file and whether Apple holds a notarization ticket for it.

usage: scripts/notarization-ticket.py <file>...     e.g. the `ys` from a release's macOS archive

Runs anywhere Python does — no macOS, no Xcode. For each architecture in each file it prints the
signing identifier, the team, and the answer of the ticket service `stapler` and Gatekeeper ask.
Exits 1 if any architecture is unsigned or has no ticket.

A ticket is looked up by the hash of the code directory (the "cdhash"), so this is the question
Gatekeeper asks about a bare executable, which cannot have a ticket stapled to it.
"""

import hashlib
import json
import struct
import sys
import urllib.request

TICKETS = "https://api.apple-cloudkit.com/database/1/com.apple.gk.ticket-delivery/production/public/records/lookup"

FAT_MAGIC = 0xCAFEBABE
LC_CODE_SIGNATURE = 0x1D
EMBEDDED_SIGNATURE = 0xFADE0CC0
CODE_DIRECTORY = 0xFADE0C02
ADHOC = 0x2
HARDENED_RUNTIME = 0x10000
SHA256 = 2
ARCHES = {0x01000007: "x86_64", 0x0100000C: "arm64"}


def c_string(data, offset):
    return data[offset : data.index(b"\0", offset)].decode()


def slices(data):
    """Each architecture of a universal binary, or the one of a thin binary."""
    if struct.unpack(">I", data[:4])[0] != FAT_MAGIC:
        yield "thin", data
        return
    (count,) = struct.unpack(">I", data[4:8])
    for i in range(count):
        cpu, _, offset, size, _ = struct.unpack(">5I", data[8 + 20 * i : 28 + 20 * i])
        yield ARCHES.get(cpu, hex(cpu)), data[offset : offset + size]


def code_directory(macho):
    """The SHA-256 code directory of a 64-bit Mach-O, or None if it is not signed."""
    (commands,) = struct.unpack("<I", macho[16:20])
    offset = 32
    for _ in range(commands):
        command, size = struct.unpack("<2I", macho[offset : offset + 8])
        if command == LC_CODE_SIGNATURE:
            start, length = struct.unpack("<2I", macho[offset + 8 : offset + 16])
            signature = macho[start : start + length]
            magic, _, blobs = struct.unpack(">3I", signature[:12])
            if magic != EMBEDDED_SIGNATURE:
                return None
            for i in range(blobs):
                _, at = struct.unpack(">2I", signature[12 + 8 * i : 20 + 8 * i])
                blob_magic, blob_length = struct.unpack(">2I", signature[at : at + 8])
                blob = signature[at : at + blob_length]
                if blob_magic == CODE_DIRECTORY and blob[37] == SHA256:
                    return blob
            return None
        offset += size
    return None


def has_ticket(cdhash):
    body = json.dumps({"records": [{"recordName": f"2/{SHA256}/{cdhash}"}]}).encode()
    request = urllib.request.Request(TICKETS, data=body, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(request, timeout=30) as response:
        record = json.load(response)["records"][0]
    return "signedTicket" in record.get("fields", {})


def main(paths):
    if not paths:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    ok = True
    for path in paths:
        print(path)
        with open(path, "rb") as file:
            data = file.read()
        for arch, macho in slices(data):
            directory = code_directory(macho)
            if directory is None:
                print(f"  {arch}: not signed")
                ok = False
                continue
            version, flags, _, identifier_at = struct.unpack(">4I", directory[8:24])
            (team_at,) = struct.unpack(">I", directory[48:52]) if version >= 0x20200 else (0,)
            team = c_string(directory, team_at) if team_at else None
            cdhash = hashlib.sha256(directory).hexdigest()[:40]
            signer = "ad-hoc signature" if flags & ADHOC else f"team {team}"
            runtime = ", hardened runtime" if flags & HARDENED_RUNTIME else ""
            notarized = has_ticket(cdhash)
            ok = ok and notarized
            print(f"  {arch}: {c_string(directory, identifier_at)}, {signer}{runtime}")
            print(f"    cdhash {cdhash}: {'notarized' if notarized else 'NO TICKET'}")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
