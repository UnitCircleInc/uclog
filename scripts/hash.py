#! /usr/bin/env python3

import argparse
from Crypto.Hash import SHA512

SIGNATURE_HDR_LEN = 512

def hash(args):
    with open(args.input, 'rb') as f:
        data = f.read()

    if args.full:
        image_data = data
    else:
        image_data = data[SIGNATURE_HDR_LEN:]
    h = SHA512.new(data=image_data).digest()

    with open(args.output, 'wb') as f:
        f.write(h)

if __name__ == '__main__':
    parser = argparse.ArgumentParser('Image Hash')

    parser.add_argument('-i', '--input', help="Input file to hash", required=True)
    parser.add_argument('-o', '--output', help="Output file", required=True)
    parser.add_argument('-f', '--full', action='store_true', help="Use full file contents")

    args = parser.parse_args()

    hash(args)
