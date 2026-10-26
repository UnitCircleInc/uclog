# © 2025 Unit Circle Inc.
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

# See https://en.wikipedia.org/wiki/Shamir's_secret_sharing

# Works with either fp.py (which is fast) or gf2.py (which is slower)

# NOTES:
#
# 1. The current implementation does not support splitting secret values
# that are larger than the field size or 0.  This can be achived by the user
# of this library by wrapping the secret with an ephemeral symmetric key
# and using a suitable symmetric AEAD encryption algorithm like
# nacl.secret.Aead.  The splitting the ephermeral key.
# [ssss](https://linux.die.net/man/1/ssss) seems to take this approach.
#
# 2. This library does not test to ensure that the results of the join
# reconstruct the orginal value.  For example if incorrect or insuffient
# secret splits are provided, an output will be computed that is incorrect.
# Detection of incorrect or insufficient secret splits when joining
# can be achived by using an MAC/HMAC hashing function on the secret and then
# verifying that the reconstructed secret's MAC/HMAC matches the original's.
# Other solutions exist depending on the application.
#
# 3. If more secret splits are provided that required for quarum, the return
# value will still be the correct secret.

# Typical Usage:
#
#  # Split a secret into 5 splits with a quarum of 3
#  field = gf2.GF2([256, 10, 5, 2, 0])
#  s = field.random()
#  while s == field(0):
#     s = field.random()
#  r = split(s, 3, 5)
#
#  # Recover secret from a quarum - could be any 3 values from r
#  rs = join(r[:3])
#
#  # s == rs


def randpoly(a0, n):
    field = a0.field
    if a0 == field(0):
        raise ValueError("a0 can't be zero")
    while True:
        p = [a0] + [field.random() for _ in range(n - 1)]
        if p[-1] != field(0):
            return p


def evalpoly(x, p):
    r = x.field(0)
    for c in p[::-1]:
        r = r * x + c
    return r


def lp_i(x, xi, xv):
    num, den = x.field(1), x.field(1)
    for xj in xv:
        num = num * (x - xj)
        den = den * (xi - xj)
    return num * den.inverse()


def lagrange(x, xy):
    xv, yv = zip(*xy)
    e = [yv[i] * lp_i(x, xv[i], xv[:i] + xv[i + 1 :]) for i in range(len(xy))]
    f = x.field(0)
    for ei in e:
        f = f + ei
    return f


def split(s, k, n):
    # s is a secret represented in a field
    # k is the quarum
    # n is to the total number of splits
    # returns [(x_fe, y_fe)] of length n
    #         x/y are the points the polynomial was evaluatated at.
    p = randpoly(s, k)
    return [(s.field(v), evalpoly(s.field(v), p)) for v in range(1, n + 1)]


def join(xy):
    # xy is [(x_fe, y_fe)] of length k - the quarum
    #       x/y are the points the polynomial was evaluatated at.
    # field is the field over which the split was created
    # returns the secret in the field
    return lagrange(xy[0][0].field(0), xy)
