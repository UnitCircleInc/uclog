#! /usr/bin/env python3

import os
import shutil
import argparse
from Crypto.Hash import SHA512

SIGNATURE_HDR_LEN = 512

def hash(input):
    with open(input, 'rb') as f:
        data = f.read()

    image_data = data[SIGNATURE_HDR_LEN:]
    return SHA512.new(data=image_data).digest()

if __name__ == '__main__':
    parser = argparse.ArgumentParser('Cache log data')

    parser.add_argument('--bin', help="FW Image binary", required=True)
    parser.add_argument('--logdata', help="Log data to cache", required=True)

    args = parser.parse_args()

    if not os.path.isfile(args.bin):
        print("Image binary not found")
        exit(1)
    if not os.path.isfile(args.logdata):
        print("Logdata file not found")
        exit(1)

    app_hash = hash(args.bin)

    user_dir = os.path.expanduser("~")
    cache_dir = os.path.join(user_dir, ".cache", "uclog")
    if not os.path.exists(cache_dir):
        os.makedirs(cache_dir)
    shutil.copy(args.logdata, os.path.join(cache_dir, app_hash.hex() + ".logdata"))
