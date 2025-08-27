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

import secrets

# secrets.randbelow is equivalent to:
# import os
#
# def getrandbits(k):
#   numbytes = (k + 7) // 8
#   x = int.from_bytes(os.urandom(numbytes))
#   return x >> (numbytes * 8 - k)
#
# def randbelow(n):
#   k = n.bit_length()
#   r = getrandbits(k)  # 0 <= r < 2**k
#   while r >= n:
#     r = getrandbits(k)
#   return r


class GF2:
    def __init__(self, poly):
        self.poly = sorted(poly)[::-1]
        self.p = sum(2**x for x in poly)
        self.n = 2 ** max(poly)

    def __call__(self, v):
        return GF2FE(v, self)

    def __repr__(self):
        return f"GF2({self.poly})"

    def __eq__(self, o):
        if self.__class__ != o.__class__:
            return False
        return self.p == o.p

    def random(self):
        return self(secrets.randbelow(self.n))


def degree(a):
    n = -1
    while a > 0:
        a = a >> 1
        n = n + 1
    return n if n > 0 else 0


def egcd(a, b):
    if a.v == 0:
        return (b, a.gf(0), a.gf(1))
    else:
        d, m = divmod(b, a)
        g, y, x = egcd(m, a)
        return (g, x - d * y, y)


class GF2FE:
    def __init__(self, v, gf):
        self.gf = gf
        self.field = gf
        if v >= gf.n and v != gf.p:
            raise ValueError(f"{v} not a member of {gf}")
        self.v = v

    def __repr__(self):
        return f"GF2FE({self.v}, {self.gf})"

    def __eq__(self, o):
        if self.__class__ != o.__class__:
            raise TypeError("Can't compare non FE values")
        return self.gf == o.gf and self.v == o.v

    def __add__(self, o):
        if self.__class__ != o.__class__:
            raise TypeError("Can't add non FE values")
        if self.gf != o.gf:
            raise TypeError("Can't add elements of different GF2 polys")
        return self.gf(self.v ^ o.v)

    def __sub__(self, o):
        # In GF(2^m) add and sub are the same
        if self.__class__ != o.__class__:
            raise TypeError("Can't sub non FE values")
        if self.gf != o.gf:
            raise TypeError("Can't sub elements of different GF2 polys")
        return self.gf(self.v ^ o.v)

    def __mul__(self, o):
        # This is (very) slow when GF2() degree is large as alg is quadratic!
        # Can be made faster by building a table or using FFT based approach
        # E.g. see https://members.loria.fr/PZimmermann/talks/ant2007.pdf p.32
        # For simple use cases with limited number of multiplies per invokation
        # this appoach is good enough and is easy to test.
        if self.__class__ != o.__class__:
            raise TypeError("Can't mult non FE values")
        if self.gf != o.gf:
            raise TypeError("Can't mult elements of different GF2 polys")
        c = 0
        a = self.v
        b = o.v
        n = self.gf.n
        p = self.gf.p
        for j in [1 << x for x in range(self.gf.poly[0])]:
            if a & j != 0:
                c = c ^ b
            b = b * 2
            if b & n != 0:
                b = b ^ p
        return self.gf(c)

    def __truediv__(self, o):
        if self.__class__ != o.__class__:
            raise TypeError("Can't mult non FE values")
        if self.gf != o.gf:
            raise TypeError("Can't mult elements of different GF2 polys")
        return self * o.inverse()

    def __divmod__(self, o):
        if self.__class__ != o.__class__:
            raise TypeError("Can't divmod non FE values")
        if self.gf != o.gf:
            raise TypeError("Can't divmod elements of different GF2 polys")

        # Long division
        # d, m = divmod(a,b)
        # a = d * b + m
        na = degree(self.v)
        nb = degree(o.v)
        r = 0
        m = 1 << na
        a = self.v
        b = o.v
        while na >= nb:
            if m & a != 0:
                r = r ^ (1 << (na - nb))
                a = a ^ (b << (na - nb))
            na = na - 1
            m = m >> 1
        return self.gf(r), self.gf(a)

    def inverse(self):
        return egcd(self.gf(self.gf.p), self)[2]
