#! /usr/bin/env python3

import argparse
from Crypto.Hash import SHA512

SIGNATURE_HDR_LEN = 512

def hash(input, output):
    with open(input, 'rb') as f:
        data = f.read()

    image_data = data[SIGNATURE_HDR_LEN:]
    h = SHA512.new(data=image_data).digest()

    with open(output, 'wb') as f:
        f.write(h)

if __name__ == '__main__':
    parser = argparse.ArgumentParser('Image Hash')

    parser.add_argument('-i', '--input', help="Input file to hash", required=True)
    parser.add_argument('-o', '--output', help="Output file", required=True)

    args = parser.parse_args()

    hash(args.input, args.output)