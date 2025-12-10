#! /usr/bin/env python3

import argparse
import hashlib

SIGNATURE_HDR_LEN = 512

def hash(args):
    with open(args.input, 'rb') as f:
        data = f.read()

    if args.full:
        image_data = data
    else:
        image_data = data[SIGNATURE_HDR_LEN:]
    hash_sha512 = hashlib.sha512()
    hash_sha512.update(image_data)
    h = hash_sha512.digest()

    with open(args.output, 'wb') as f:
        f.write(h)

if __name__ == '__main__':
    parser = argparse.ArgumentParser('Image Hash')

    parser.add_argument('-i', '--input', help="Input file to hash", required=True)
    parser.add_argument('-o', '--output', help="Output file", required=True)
    parser.add_argument('-f', '--full', action='store_true', help="Use full file contents")

    args = parser.parse_args()

    hash(args)
