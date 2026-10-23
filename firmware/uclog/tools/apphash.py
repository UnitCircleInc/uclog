#!/usr/bin/env python3
# © 2025 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0
"""SHA-512 of a firmware image (unsigned images: the whole file, with the .apphash field still
zero). Writes the 64-byte digest to OUT and prints it in hex."""

import argparse
import hashlib

ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
ap.add_argument('bin')
ap.add_argument('-o', '--output', required=True)
a = ap.parse_args()
with open(a.bin, 'rb') as f:
  digest = hashlib.sha512(f.read()).digest()
with open(a.output, 'wb') as f:
  f.write(digest)
print(digest.hex())
