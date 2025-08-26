// © 2016-2022 Unit Circle Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

// The following flags can be used to create smaller executables
// CBOR_NO_UTF8            - disables UTF8 checking on reading TEXT
// CBOR_NO_DATETIME_STRING - disables handling of TAG(0) date/time decoding
// CBOR_NO_RATIONAL        - disables handling of TAG(30) rational decoding
// CBOR_NO_DECIMAL         - disables handling of TAG(4) decimal decoding
// CBOR_NO_ENCODED         - disables handling of TAG(24) encoded decoding
// CBOR_NO_FLOAT           - disables float support
// CBOR_NO_DATETIME        - disables datetime support

#if !defined(CBOR_NO_DATETIME_STRING)
#define CBOR_NO_DATETIME_STRING
#endif
#if !defined(CBOR_NO_DECIMAL)
#define CBOR_NO_DECIMAL
#endif

#define CBOR_MAX_RECURSION (4)

#include <stdlib.h>
#include <stdint.h>
#include <stdbool.h>
#include <string.h>
#include <math.h>
#if !defined(CBOR_NO_DATETIME_STRING)
#include <time.h>
#endif
#include <cbor.h>
#include <log.h>

#if !defined(CBOR_NO_DECIMAL)
float64_t mult_pow10(float64_t m, int exp) {
  return m*pow(10., exp);
}
#endif

// Reading Arrays
// Generally we deal with small arrays of fixed types
// and we want all values - so the following is reasonable
//   cbor_read_array(x, &s, &n);
//   for (size_t i = 0; i < n; i++) {
//     cbor_read_int64(&s, &n);
//   }
//
// and thus we don't need:
//   cbor_idx_any(cbor_stream_t*s, size_t idx, cbor_value_t* v);
//   cbor_idx_XXX(cbor_stream_t*s, size_t idx, XXX* v);
//
// until we have support for CBOR Typed Arrays
// (see https://tools.ietf.org/html/draft-ietf-cbor-array-tags-08)
// With Type Array support we could provide direct access or something
// similar to:
//   cbor_error_t cbor_typed_array_move(cbor_stream_t*, XXX* v, size_t n);

#if !defined(CBOR_NO_UTF8)
#include "utf8valid.h"
#endif
#if !defined(CBOR_NO_DATETIME_STRING)
#include <stdarg.h>
#include <limits.h>
#endif

#define RET_ERROR(s, e) do { \
  s->error = e; \
  return e; \
} while (false)

#define CHECK_ERROR(s) do { \
  if (s->error != CBOR_ERROR_NONE) return s->error; \
} while (false)

#define CHECK(x) do { \
  cbor_error_t e = x; \
  if (e != CBOR_ERROR_NONE) { \
    return e; \
  } \
} while(false)


static cbor_error_t read_ext(cbor_stream_t* s, uint8_t* mt, uint8_t* ai, uint64_t* v) {
  if (s->n < 1) RET_ERROR(s, CBOR_ERROR_END_OF_STREAM);
  *mt = s->b[0] >> 5;
  *ai = s->b[0] & 0x1f;
  s->b++; s->n--;

  if (*ai < 24) {
    *v = *ai;
  }
  else if (*ai < 28) {
    size_t n = 1U << (*ai-24U);

    if (s->n < n) RET_ERROR(s, CBOR_ERROR_END_OF_STREAM);
    *v = *(s->b++); s->n--; n--;
    while (n > 0) {
      *v = (*v << 8) + (*(s->b)++); s->n--; n--;
    }
  }
  else if ((*ai == 31) && (*mt != 0) && (*mt != 1) && (*mt != 6)) {
    // variable length string, bytes, array, map, and break are ok
  }
  else {
    RET_ERROR(s, CBOR_ERROR_INVALID_AI);
  }
  return CBOR_ERROR_NONE;
}

enum op_e {
  OP_LEN,
  OP_CPY,
  OP_CMP,
};

typedef struct {
  enum op_e op;
  size_t    n;
  void*     b;
  int32_t   r;
} op_t;


static cbor_error_t read_bytes_like(cbor_stream_t* s, uint8_t mt, uint8_t ai, uint64_t n, op_t* op) {
  bool multi = ai == 31;
  uint8_t original_mt = mt;
  do {
    if (multi) {
      CHECK(read_ext(s, &mt, &ai, &n));
      if ((mt == 7) && (ai == 31)) break;
      if (mt != original_mt) RET_ERROR(s, CBOR_ERROR_INDEF_MISMATCH);
      if (ai == 31) RET_ERROR(s, CBOR_ERROR_INDEF_NESTING);
    }

    if (s->n < n) RET_ERROR(s, CBOR_ERROR_END_OF_STREAM);

    if (op->op == OP_LEN) {
#if !defined(CBOR_NO_UTF8)
      if ((mt == 3) && (!is_valid_utf8((const char*) s->b, (size_t) n))) {
        RET_ERROR(s, CBOR_ERROR_INVALID_UTF8);
      }
#endif
      op->n += n;
    }
    else {
      if (op->n < n) RET_ERROR(s, CBOR_ERROR_BUFFER_TOO_SMALL);
      if (op->op == OP_CPY) {
        memmove(op->b, s->b, (size_t) n);
      }
      else {
        op->r = memcmp(op->b, s->b, (size_t) n);
        if (op->r != 0) return CBOR_ERROR_NONE;
      }
      op->n -= n;
      op->b = ((uint8_t*) op->b) + n;
    }

    s->b += n;
    s->n -= n;
  } while (multi);
  return CBOR_ERROR_NONE;
}

cbor_error_t cbor_memmove(void* b, cbor_stream_t* s, size_t nb) {
  CHECK_ERROR(s);
  uint8_t mt;
  uint8_t ai;
  uint64_t n;
  CHECK(read_ext(s, &mt, &ai, &n));

  if ((mt != 2) && (mt != 3)) RET_ERROR(s, CBOR_ERROR_BAD_TYPE);

  op_t op = { .op = OP_CPY, .n = nb, .b = b, .r = 0 };
  return read_bytes_like(s, mt, ai, n, &op);
}

// Note for return values > 255 - in whichcase return vaue - 256 == error code
int cbor_memcmp(const void* b, cbor_stream_t* s, size_t nb) {
  CHECK_ERROR(s);
  uint8_t mt;
  uint8_t ai;
  uint64_t n;
  CHECK(read_ext(s, &mt, &ai, &n));

  if ((mt != 2) && (mt != 3)) RET_ERROR(s, 256 + CBOR_ERROR_BAD_TYPE);

  op_t op = { .op = OP_CMP, .n = nb, .b = (void*) b, .r = 0 };
  cbor_error_t e = read_bytes_like(s, mt, ai, n, &op);
  if (e != CBOR_ERROR_NONE) RET_ERROR(s, 256 + e);
  return op.r;
}

int cbor_strcmp(const char* t, cbor_stream_t* s) {
  return cbor_memcmp(t, s, strlen(t));
}

cbor_error_t cbor_as_int64(const cbor_value_t* v, int64_t* r) {
  if (v->type == CBOR_TYPE_UINT) {
    if (v->value.uint_v > 0x7fffffffffffffffULL) return CBOR_ERROR_RANGE;
    *r = (int64_t) v->value.uint_v;
  }
  else if (v->type == CBOR_TYPE_NINT) {
    if (v->value.nint_v > 0x7fffffffffffffffULL) return CBOR_ERROR_RANGE;
    *r = (int64_t) v->value.nint_v;
    *r = -(*r) - 1;
  }
  else {
    return CBOR_ERROR_CANT_CONVERT_TYPE;
  }
  return CBOR_ERROR_NONE;
}

cbor_error_t cbor_as_int32(cbor_value_t*v, int32_t* r) {
  int64_t rr;
  CHECK(cbor_as_int64(v, &rr));
  if (rr >  0x7fffffffLL) return CBOR_ERROR_RANGE;
  if (rr < -0x80000000LL) return CBOR_ERROR_RANGE;
  *r = (int32_t) rr;
  return CBOR_ERROR_NONE;
}

cbor_error_t cbor_as_int16(cbor_value_t*v, int16_t* r) {
  int64_t rr;
  CHECK(cbor_as_int64(v, &rr));
  if (rr >  0x7fff) return CBOR_ERROR_RANGE;
  if (rr < -0x8000) return CBOR_ERROR_RANGE;
  *r = (int16_t) rr;
  return CBOR_ERROR_NONE;
}

cbor_error_t cbor_as_int8(const cbor_value_t*v, int8_t* r) {
  int64_t rr;
  CHECK(cbor_as_int64(v, &rr));
  if (rr >  0x7f) return CBOR_ERROR_RANGE;
  if (rr < -0x80) return CBOR_ERROR_RANGE;
  *r = (int8_t) rr;
  return CBOR_ERROR_NONE;
}

cbor_error_t cbor_as_uint64(const cbor_value_t* v, uint64_t* r) {
  if (v->type != CBOR_TYPE_UINT) return CBOR_ERROR_CANT_CONVERT_TYPE;
  *r = v->value.uint_v;
  return CBOR_ERROR_NONE;
}

cbor_error_t cbor_as_uint32(const cbor_value_t* v, uint32_t* r) {
  uint64_t rr;
  CHECK(cbor_as_uint64(v, &rr));
  if (rr > 0xffffffffULL) return CBOR_ERROR_RANGE;
  *r = (uint32_t) rr;
  return CBOR_ERROR_NONE;
}

cbor_error_t cbor_as_uint16(const cbor_value_t* v, uint16_t* r) {
  uint64_t rr;
  CHECK(cbor_as_uint64(v, &rr));
  if (rr > 0xffffu) return CBOR_ERROR_RANGE;
  *r = (uint16_t) rr;
  return CBOR_ERROR_NONE;
}

cbor_error_t cbor_as_uint8(const cbor_value_t* v, uint8_t* r) {
  uint64_t rr;
  CHECK(cbor_as_uint64(v, &rr));
  if (rr > 0xffu) return CBOR_ERROR_RANGE;
  *r = (uint8_t) rr;
  return CBOR_ERROR_NONE;
}

cbor_error_t cbor_as_bool(const cbor_value_t* v, bool* r) {
  if (v->type != CBOR_TYPE_BOOL) return CBOR_ERROR_CANT_CONVERT_TYPE;
  *r = v->value.bool_v;
  return CBOR_ERROR_NONE;
}

cbor_error_t cbor_as_null(const cbor_value_t* v) {
  if (v->type != CBOR_TYPE_NULL) return CBOR_ERROR_CANT_CONVERT_TYPE;
  return CBOR_ERROR_NONE;
}

cbor_error_t cbor_as_undefined(const cbor_value_t* v) {
  if (v->type != CBOR_TYPE_UNDEFINED) return CBOR_ERROR_CANT_CONVERT_TYPE;
  return CBOR_ERROR_NONE;
}

cbor_error_t cbor_as_simple(const cbor_value_t* v, uint8_t* r) {
  if (v->type != CBOR_TYPE_SIMPLE) return CBOR_ERROR_CANT_CONVERT_TYPE;
  *r = v->value.simple_v;
  return CBOR_ERROR_NONE;
}

#if !defined(CBOR_NO_DECIMAL)
cbor_error_t cbor_as_decimal(const cbor_value_t* v, int64_t* mant, int64_t* exp) {
  if (v->type != CBOR_TYPE_DECIMAL) return CBOR_ERROR_CANT_CONVERT_TYPE;
  *mant  = v->value.decimal_v.mant;
  *exp  = v->value.decimal_v.exp;
  return CBOR_ERROR_NONE;
}
#endif

#if !defined(CBOR_NO_RATIONAL)
cbor_error_t cbor_as_rational(const cbor_value_t* v, int64_t* n, uint64_t* d) {
  if (v->type != CBOR_TYPE_RATIONAL) return CBOR_ERROR_CANT_CONVERT_TYPE;
  *n  = v->value.rational_v.n;
  *d  = v->value.rational_v.d;
  return CBOR_ERROR_NONE;
}
#endif

#if !defined(CBOR_NO_FLOAT)
cbor_error_t cbor_as_float64(const cbor_value_t* v, float64_t* d) {
  switch (v->type) {
    case CBOR_TYPE_UINT:    *d = (float64_t) v->value.uint_v; return CBOR_ERROR_NONE;
    case CBOR_TYPE_NINT:    *d = -1;
                            *d-= (float64_t) v->value.nint_v; return CBOR_ERROR_NONE;
    case CBOR_TYPE_FLOAT16: *d = v->value.float16_v; return CBOR_ERROR_NONE;
    case CBOR_TYPE_FLOAT32: *d = v->value.float32_v; return CBOR_ERROR_NONE;
    case CBOR_TYPE_FLOAT64: *d = v->value.float64_v; return CBOR_ERROR_NONE;
#if !defined(CBOR_NO_DECIMAL)
    case CBOR_TYPE_DECIMAL: {
      int64_t mant;
      int64_t exp;
      CHECK(cbor_as_decimal(v, &mant, &exp));
      if (mant == 0) {
        *d = 0.0;
      }
      else if (exp > 3000) {
        *d = INFINITY;
      }
      else if (exp < -3000) {
        *d = mant * 0.0;
      }
      else {
        *d = mult_pow10(mant, (int) exp);
      }
      return CBOR_ERROR_NONE;
    }
#endif
#if !defined(CBOR_NO_RATIONAL)
    case CBOR_TYPE_RATIONAL: {
      int64_t num;
      uint64_t denom;
      CHECK(cbor_as_rational(v, &num, &denom));
      *d = (float64_t) num;
      *d = *d / (float64_t) denom;
      return CBOR_ERROR_NONE;
    }
#endif
    default: return CBOR_ERROR_BAD_FLOAT64;
  }
}

cbor_error_t cbor_as_float32(const cbor_value_t* v, float32_t* f) {
  float64_t d;
  CHECK(cbor_as_float64(v, &d));
  *f = (float32_t) d;
  return CBOR_ERROR_NONE;
}

cbor_error_t cbor_as_float16(const cbor_value_t* v, float16_t* f) {
  float64_t d;
  CHECK(cbor_as_float64(v, &d));
  *f = (float16_t) d;
  return CBOR_ERROR_NONE;
}
#endif

cbor_error_t cbor_as_datetime(const cbor_value_t* v, float64_t* d) {
  if (v->type != CBOR_TYPE_DATETIME) return CBOR_ERROR_CANT_CONVERT_TYPE;
  *d = v->value.datetime_v;
  return CBOR_ERROR_NONE;
}

cbor_error_t cbor_as_tag(const cbor_value_t* v, cbor_stream_t* m, uint64_t* tag) {
  if (v->type != CBOR_TYPE_TAG) return CBOR_ERROR_CANT_CONVERT_TYPE;
  *m = v->value.tag_v.s;
  *tag = v->value.tag_v.tag;
  return CBOR_ERROR_NONE;
}

cbor_error_t cbor_as_stream_like(const cbor_value_t* v, cbor_type_t ty, cbor_stream_t* m, size_t* n) {
  if (v->type != ty) return CBOR_ERROR_CANT_CONVERT_TYPE;
  *m = v->value.stream_v.s;
  *n = v->value.stream_v.n;
  return CBOR_ERROR_NONE;
}


#define CBOR_AS_STREAM_TYPE(name, ty) \
cbor_error_t cbor_as_ ## name(const cbor_value_t* v, cbor_stream_t* m, size_t* n) { \
  return cbor_as_stream_like(v, ty, m, n); \
}

CBOR_AS_STREAM_TYPE(encoded, CBOR_TYPE_ENCODED)
CBOR_AS_STREAM_TYPE(text, CBOR_TYPE_TEXT)
CBOR_AS_STREAM_TYPE(bytes, CBOR_TYPE_BYTES)
CBOR_AS_STREAM_TYPE(array, CBOR_TYPE_ARRAY)
CBOR_AS_STREAM_TYPE(map, CBOR_TYPE_MAP)

#if !defined(CBOR_NO_DATETIME_STRING)
static unsigned cvt_unsigned(const char*s, char** e, unsigned digits) {
  unsigned r = 0;
  *e = (char*)s;
  while (digits-- > 0) {
    if ((*s < '0') || (*s > '9')) return 0;
    r = r * 10 + ((*s++) - '0');
  }
  *e = (char*)s;
  return r;
}

static int parse_rfc3339(const char* s, const char* fmt, ...) {
  va_list args;
  size_t n = 0;
  char* new_s;
  va_start(args, fmt);
  while ((*fmt != '\0') && (*s != '\0')) {
    if (*fmt == '%') {
      fmt++;
      if ((*fmt == 'u') || (*fmt == 'U')) {
        unsigned* v = va_arg(args, unsigned*);
        *v = cvt_unsigned(s, &new_s, *fmt == 'u' ? 2 : 4);
        if (new_s == s) break;
        s = new_s;
      }
      else if (*fmt == 'f') {
        float64_t* v = va_arg(args, float64_t*);
        unsigned d = cvt_unsigned(s, &new_s, 2);
        if (new_s == s) break;
        s = new_s;
        *v = d;
        if (*s == '.') {
          float64_t f = 0.1;
          s++;
          if ((*s < '0') || (*s > '9')) break;
          do {
            *v += (*s++ - '0') * f;  f *= 0.1;
          } while ((*s >= '0') && (*s <= '9'));
        }
      }
      else if (*fmt == 'c') {
        char* v = va_arg(args, char*);
        *v = *s;
         s++;
      }
      else {
        break;
      }
      fmt++;
      n++;
    }
    else if (*fmt == *s) {
      fmt++; s++;
    }
    else {
      break;
    }
  }
  if (*s != '\0') return 0; // We must reach end of string
  return n;
}

static cbor_error_t cvt_datetime_string(cbor_value_t* v, cbor_value_t* v2) {
  struct tm ctime;
  memset(&ctime, 0, sizeof(struct tm));
  unsigned year;
  unsigned month;
  unsigned day;
  unsigned hour;
  unsigned min;
  float64_t sec;
  char     tzchar;
  unsigned tzhour;
  unsigned tzmin;
  int      tzoffset = 0;

  // Get v2 as a text string
  char text[40];
  cbor_stream_t s;
  size_t n_s;
  CHECK(cbor_as_text(v2, &s, &n_s));
  if (n_s >= 40) return CBOR_ERROR_RANGE;
  memset(text, 0, 40);
  CHECK(cbor_memmove(text, &s, sizeof(text)-1));

  // Parse the string
  int n = parse_rfc3339(text, "%U-%u-%uT%u:%u:%f%c%u:%u",
      &year, &month, &day, &hour, &min, &sec, &tzchar, &tzhour, &tzmin);

  //printf("n: %d %u-%u-%uT%u:%u:%f%c%u:%u\n", n,
  //    year, month, day, hour, min, sec, tzchar, tzhour, tzmin);

  // Check that it parsed correctly
  if (n >= 7) {
    if ((year  < 1900) || (year  > 9999) ||
        (month <    1) || (month >   12) ||
        (day   <    1) || (day   >   31) ||
        (hour > 23) || (min > 59) ||
        (sec > 60.0))  return CBOR_ERROR_RANGE;
  }

  if (n == 7) {
    if (tzchar != 'Z') return CBOR_ERROR_RANGE;
  }
  else if (n == 9) {
   if ((tzchar != '+') && (tzchar != '-')) return CBOR_ERROR_RANGE;
   if ((tzhour > 23) || (tzmin > 59)) return CBOR_ERROR_RANGE;
   tzoffset = (tzchar == '+' ? 1 : -1) * (tzhour * 3600 + tzmin * 60);
  }
  else {
    return CBOR_ERROR_RANGE;
  }

  // Convert to seconds since epoc
  ctime.tm_year = year - 1900;
  ctime.tm_mon  = month - 1;
  ctime.tm_mday = day;
  ctime.tm_hour = hour;
  ctime.tm_min  = min;
  ctime.tm_sec  = 0;

  v->type = CBOR_TYPE_DATETIME;
  v->value.datetime_v = mktime(&ctime) + sec - tzoffset;
  return CBOR_ERROR_NONE;
}

#endif

#if !defined(CBOR_NO_DATETIME)
static cbor_error_t cvt_datetime_number(cbor_value_t* v, cbor_value_t* v2) {
  v->type = CBOR_TYPE_DATETIME;
  return cbor_as_float64(v2, &(v->value.datetime_v));
}
#endif


#if !defined(CBOR_NO_DECIMAL) || !defined(CBOR_NO_RATIONAL)
static cbor_error_t cvt_array_n(cbor_value_t* v, cbor_value_t* a, size_t n, cbor_error_t eret) {
  cbor_stream_t s;

  if (v->type != CBOR_TYPE_ARRAY) return eret;
  if (v->value.stream_v.n != n) return eret;
  s = v->value.stream_v.s;
  while (n-- > 0) {
    CHECK(cbor_read_any(&s, a));
    a++;
  }
  return CBOR_ERROR_NONE;
}
#endif

#if !defined(CBOR_NO_DECIMAL)
static cbor_error_t cvt_decimal(const cbor_value_t* v, cbor_value_t* v2) {
  cbor_value_t me[2];
  v->type = CBOR_TYPE_DECIMAL;
  CHECK(cvt_array_n(v2, me , 2, CBOR_ERROR_BAD_DECIMAL));
  CHECK(cbor_as_int64(me+1, &(v->value.decimal_v.mant)));
  return cbor_as_int64(me+0, &(v->value.decimal_v.exp));
}
#endif

#if !defined(CBOR_NO_RATIONAL)
static cbor_error_t cvt_rational(cbor_value_t* v, cbor_value_t* v2) {
  cbor_value_t fr[2];
  v->type = CBOR_TYPE_RATIONAL;
  CHECK(cvt_array_n(v2, fr , 2, CBOR_ERROR_BAD_RATIONAL));
  CHECK(cbor_as_uint64(fr+1, &(v->value.rational_v.d)));
  if (v->value.rational_v.d == 0) return CBOR_ERROR_BAD_RATIONAL;
  return cbor_as_int64(fr+0, &(v->value.rational_v.n));
}
#endif

#if !defined(CBOR_NO_ENCODED)
static cbor_error_t cvt_encoded(cbor_value_t*v, cbor_value_t*  v2) {
  if (v2->type != CBOR_TYPE_BYTES) return CBOR_ERROR_BAD_ENCODED;
  v->type = CBOR_TYPE_ENCODED;
  v->value.stream_v = v2->value.stream_v;
  return CBOR_ERROR_NONE;
}
#endif

static cbor_error_t read_any(cbor_stream_t* s, cbor_value_t* v, size_t depth) {
  cbor_value_t my_v;
  union {
    uint64_t v;
    float16_t f16;
    float32_t f32;
    float64_t f64;
  } int_to_float;

  if (depth > CBOR_MAX_RECURSION) RET_ERROR(s, CBOR_ERROR_RECURSION);

read1:;
  // Record current position
  uint8_t* start_b = s->b;

  uint8_t mt;
  uint8_t ai;
  uint64_t n;
  CHECK(read_ext(s, &mt, &ai, &n));
  //printf("mt: %u ai: %u n: %llu depth: %zu\n", mt, ai, n, depth);

  switch (mt & 7) {
    case 0: // Unsigned int
      v->type = CBOR_TYPE_UINT;
      v->value.uint_v = n;
      return CBOR_ERROR_NONE;

    case 1: // Negative int
      v->type = CBOR_TYPE_NINT;
      v->value.nint_v = n;
      return CBOR_ERROR_NONE;

    case 2: // Byte string
    case 3: // Text string
      v->type = mt == 2 ? CBOR_TYPE_BYTES : CBOR_TYPE_TEXT;
      op_t op = { .op = OP_LEN, .n = 0, .b = NULL, .r = 0 };
      CHECK(read_bytes_like(s, mt, ai, n, &op));
      v->value.stream_v.s.b = start_b;
      v->value.stream_v.s.n = s->b - start_b;
      v->value.stream_v.s.error = CBOR_ERROR_NONE;
      v->value.stream_v.n = op.n;
      return CBOR_ERROR_NONE;

    case 4: // Array
    case 5: // Map
      v->type = mt == 4 ? CBOR_TYPE_ARRAY : CBOR_TYPE_MAP;
      v->value.stream_v.s.error = CBOR_ERROR_NONE;
      if (ai == 31) {
        v->value.stream_v.s.b = s->b;
        v->value.stream_v.n = 0;
        while (true) {
          if (s->n < 1) RET_ERROR(s, CBOR_ERROR_END_OF_STREAM);
          mt = s->b[0] >> 5;
          ai = s->b[0] & 0x1f;
          //printf("  mt: %u ai: %u depth: %zu\n", mt, ai, depth);
          if ((mt == 7) && (ai == 31)) break;
          CHECK(read_any(s, &my_v, depth+1));
          v->value.stream_v.n += 1;
        }
        v->value.stream_v.s.n = s->b - v->value.stream_v.s.b;
        s->b++; s->n--;

        if (v->type == CBOR_TYPE_MAP) {
          if (v->value.stream_v.n % 2 != 0) RET_ERROR(s, CBOR_ERROR_MAP_LENGTH);
          v->value.stream_v.n >>= 1;
        }
      }
      else {
        v->value.stream_v.s.b = s->b;
        if (n > SIZE_MAX) RET_ERROR(s, CBOR_ERROR_ITEM_TOO_LONG);
        v->value.stream_v.n = (size_t) n;
        if (mt == 5) n += n;
        while (n-- > 0) {
          CHECK(read_any(s, &my_v, depth+1));
        }
        v->value.stream_v.s.n = s->b - v->value.stream_v.s.b;
      }
      return CBOR_ERROR_NONE;

    case 6:
      v->type = CBOR_TYPE_TAG;
      v->value.tag_v.tag = n;

      // Special case - strip CBOR marker
      if (v->value.tag_v.tag == 55799) goto read1;

      v->value.tag_v.s = *s;              // Error propogates
      CHECK(read_any(s, &my_v, depth+1)); // Get tagged item

      // Convert "known" tagged types
#if !defined(CBOR_NO_DATETIME)
#if !defined(CBOR_NO_DATETIME_STRING)
      if (v->value.tag_v.tag == 0) {
        return cvt_datetime_string(v, &my_v);
      }
#endif
      if (v->value.tag_v.tag == 1) {
        return cvt_datetime_number(v, &my_v);
      }
#endif
#if !defined(CBOR_NO_DECIMAL)
      if (v->value.tag_v.tag == 4) {
        return cvt_decimal(v, &my_v);
      }
#endif
#if !defined(CBOR_NO_ENCODED)
      if (v->value.tag_v.tag == 24) {
        return cvt_encoded(v, &my_v);
      }
#endif
#if !defined(CBOR_NO_RATIONAL)
      if (v->value.tag_v.tag == 30) {
        return cvt_rational(v, &my_v);
      }
#endif
      return CBOR_ERROR_NONE;

    case 7: // Simple
      switch (ai) {
        case 20:
        case 21:
          v->type = CBOR_TYPE_BOOL;
          v->value.bool_v = ai == 21;
          return CBOR_ERROR_NONE;
        case 22:
          v->type = CBOR_TYPE_NULL;
          return CBOR_ERROR_NONE;
        case 23:
          v->type = CBOR_TYPE_UNDEFINED;
          return CBOR_ERROR_NONE;
        case 24:
          if (n > 255) RET_ERROR(s, CBOR_ERROR_BAD_SIMPLE_VALUE);
          if (n < 32) RET_ERROR(s, CBOR_ERROR_BAD_SIMPLE_VALUE);
          v->type = CBOR_TYPE_SIMPLE;
          v->value.simple_v = (uint8_t) n;
          return CBOR_ERROR_NONE;
#if !defined(CBOR_NO_FLOAT16)
        case 25:
          v->type = CBOR_TYPE_FLOAT16;
          int_to_float.v = n;
          v->value.float16_v = int_to_float.f16;
          return CBOR_ERROR_NONE;
#endif
        case 26:
          v->type = CBOR_TYPE_FLOAT32;
          int_to_float.v = n;
          v->value.float32_v = int_to_float.f32;
          return CBOR_ERROR_NONE;
        case 27:
          v->type = CBOR_TYPE_FLOAT64;
          int_to_float.v = n;
          v->value.float64_v = int_to_float.f64;
          return CBOR_ERROR_NONE;
        case 28:
        case 29:
        case 30:
          RET_ERROR(s, CBOR_ERROR_BAD_SIMPLE_VALUE);
        case 31:
          RET_ERROR(s, CBOR_ERROR_UNEXPECTED_BREAK);
        default:
          v->type = CBOR_TYPE_SIMPLE;
          v->value.simple_v = ai;
          return CBOR_ERROR_NONE;
      }
  }
  RET_ERROR(s, CBOR_ERROR_INTERNAL_1);
}

cbor_error_t cbor_read_any(cbor_stream_t* s, cbor_value_t* v) {
  if (s == NULL) return CBOR_ERROR_NULL;
  if (s->b == NULL) return CBOR_ERROR_NULL;
  CHECK_ERROR(s);
  CHECK(read_any(s, v, 0));
  return CBOR_ERROR_NONE;
}

#define CBOR_READ_0(name) \
  cbor_error_t cbor_read_ ## name(cbor_stream_t* s) { \
    cbor_value_t v; \
    CHECK(cbor_read_any(s, &v)); \
    return cbor_as_ ## name(&v); \
  }

#define CBOR_READ_1(name, type) \
  cbor_error_t cbor_read_ ## name(cbor_stream_t* s, type* u) { \
    cbor_value_t v; \
    CHECK(cbor_read_any(s, &v)); \
    return cbor_as_ ## name(&v, u); \
  }

#define CBOR_READ_2(name, type1, type2) \
  cbor_error_t cbor_read_ ## name(cbor_stream_t* s, type1* r1, type2* r2) { \
    cbor_value_t v; \
    CHECK(cbor_read_any(s, &v)); \
    return cbor_as_ ## name(&v, r1, r2); \
  }


CBOR_READ_1(uint64, uint64_t)
CBOR_READ_1(uint32, uint32_t)
CBOR_READ_1(uint16, uint16_t)
CBOR_READ_1(uint8, uint8_t)
CBOR_READ_1(int64, int64_t)
CBOR_READ_1(int32, int32_t)
CBOR_READ_1(int16, int16_t)
CBOR_READ_1(int8, int8_t)
CBOR_READ_1(bool, bool)
CBOR_READ_0(null)
CBOR_READ_0(undefined)
CBOR_READ_1(simple, uint8_t)
#if !defined(CBOR_NO_DECIMAL)
CBOR_READ_2(decimal, int64_t, int64_t)
#endif
#if !defined(CBOR_NO_RATIONAL)
CBOR_READ_2(rational, int64_t, uint64_t)
#endif
CBOR_READ_1(float64, float64_t)
CBOR_READ_1(float32, float32_t)
CBOR_READ_1(float16, float16_t)
CBOR_READ_1(datetime, float64_t)
CBOR_READ_2(tag, cbor_stream_t, uint64_t)
#if !defined(CBOR_NO_ENCODED)
CBOR_READ_2(encoded, cbor_stream_t, size_t)
#endif
CBOR_READ_2(text, cbor_stream_t, size_t)
CBOR_READ_2(bytes, cbor_stream_t, size_t)
CBOR_READ_2(array, cbor_stream_t, size_t)
CBOR_READ_2(map, cbor_stream_t, size_t)

cbor_error_t cbor_get_any(cbor_stream_t *s, size_t n,
                               const char* k, cbor_value_t* v) {
  cbor_stream_t s2 = *s;
  size_t key_n = strlen(k);
  while (n-- > 0) {
    CHECK(cbor_read_any(&s2, v));
    if (v->type == CBOR_TYPE_TEXT) {
      if (v->value.stream_v.n == key_n) {
        if (cbor_memcmp(k, &(v->value.stream_v.s), key_n) == 0) {
          return cbor_read_any(&s2, v);
        }
      }
    }
    CHECK(cbor_read_any(&s2, v));
  }
  return CBOR_ERROR_KEY_NOT_FOUND;
}

#if 0
// This can be enabled if there is a need for "keys of any type".
static bool eq_values(const cbor_value_t* v1, const cbor_value_t* v2) {
  if (v1->type != v2->type) return false;
  switch (v1->type) {
    case CBOR_TYPE_NINT: return v1->value.nint_v == v2->value.nint_v;
    case CBOR_TYPE_UINT: return v1->value.uint_v == v2->value.uint_v;
    case CBOR_TYPE_BOOL: return v1->value.bool_v == v2->value.bool_v;
    ...
  }
}

cbor_error_t cbor_map_get_any_any(cbor_stream_t*s size_t n,
                                  const cbor_value_t* k, cbor_value_t* v) {
  cbor_stream_t s2 = *s;
  size_t key_n = strlen(k);
  while (n-- > 0) {
    CHECK(cbor_read_any(&s2, v));
    if (eq_values(k, v)) {
      return cbor_read_any(&s2, v);
    }
    CHECK(cbor_read_any(&s2, v));
  }
  return CBOR_ERROR_KEY_NOT_FOUND;
}


// This can be included is we specifically want include "integer keys".
// Useful for key compression
cbor_error_t cbor_map_idx_any(cbor_stream_t *s, size_t n,
                             int64_t k, cbor_value_t* v) {
  cbor_stream_t s2 = *s;
  int64_t kk;
  while (n-- > 0) {
    CHECK(cbor_read_int64(&s2, &kk));
    if (k == kk) {
      return cbor_read_any(&s2, v);
    }
    CHECK(cbor_read_any(&s2, v));
  }
  return CBOR_ERROR_KEY_NOT_FOUND;
}

#endif

#define CBOR_GET_0(name) \
  cbor_error_t cbor_get_ ## name(cbor_stream_t* s, size_t n, const char* k) { \
    cbor_value_t v; \
    CHECK(cbor_get_any(s, n, k, &v)); \
    return cbor_as_ ## name(&v); \
  }

#define CBOR_GET_1(name, type) \
  cbor_error_t cbor_get_ ## name(cbor_stream_t* s, size_t n, const char* k, type* u) { \
    cbor_value_t v; \
    CHECK(cbor_get_any(s, n, k, &v)); \
    return cbor_as_ ## name(&v, u); \
  }

#define CBOR_GET_2(name, type1, type2) \
  cbor_error_t cbor_get_ ## name(cbor_stream_t* s, size_t n, const char* k, type1* r1, type2* r2) { \
    cbor_value_t v; \
    CHECK(cbor_get_any(s, n, k, &v)); \
    return cbor_as_ ## name(&v, r1, r2); \
  }

CBOR_GET_1(uint64, uint64_t)
CBOR_GET_1(uint32, uint32_t)
CBOR_GET_1(uint16, uint16_t)
CBOR_GET_1(uint8, uint8_t)
CBOR_GET_1(int64, int64_t)
CBOR_GET_1(int32, int32_t)
CBOR_GET_1(int16, int16_t)
CBOR_GET_1(int8, int8_t)
CBOR_GET_1(bool, bool)
CBOR_GET_0(null)
CBOR_GET_0(undefined)
CBOR_GET_1(simple, uint8_t)
#if !defined(CBOR_NO_DECIMAL)
CBOR_GET_2(decimal, int64_t, int64_t)
#endif
#if !defined(CBOR_NO_RATIONAL)
CBOR_GET_2(rational, int64_t, uint64_t)
#endif
CBOR_GET_1(float64, float64_t)
CBOR_GET_1(datetime, float64_t)
CBOR_GET_2(tag, cbor_stream_t, uint64_t)
#if !defined(CBOR_NO_ENCODED)
CBOR_GET_2(encoded, cbor_stream_t, size_t)
#endif
CBOR_GET_2(text, cbor_stream_t, size_t)
CBOR_GET_2(bytes, cbor_stream_t, size_t)
CBOR_GET_2(array, cbor_stream_t, size_t)
CBOR_GET_2(map, cbor_stream_t, size_t)


cbor_error_t cbor_idx_any(const cbor_stream_t *s, size_t n,
                             size_t idx, cbor_value_t* v) {
  if (idx >= n) return CBOR_ERROR_IDX_TOO_BIG;
  cbor_stream_t s2 = *s;
  do {
    CHECK(cbor_read_any(&s2, v));
    if (idx == 0) return CBOR_ERROR_NONE;
    idx--;
  } while (true);
}


#define CBOR_IDX_0(name) \
  cbor_error_t cbor_idx_ ## name(const cbor_stream_t* s, size_t n, size_t idx) { \
    cbor_value_t v; \
    CHECK(cbor_idx_any(s, n, idx, &v)); \
    return cbor_as_ ## name(&v); \
  }

#define CBOR_IDX_1(name, type) \
  cbor_error_t cbor_idx_ ## name(const cbor_stream_t* s, size_t n, size_t idx, type* u) { \
    cbor_value_t v; \
    CHECK(cbor_idx_any(s, n, idx, &v)); \
    return cbor_as_ ## name(&v, u); \
  }

#define CBOR_IDX_2(name, type1, type2) \
  cbor_error_t cbor_idx_ ## name(const cbor_stream_t* s, size_t n, size_t idx, type1* r1, type2* r2) { \
    cbor_value_t v; \
    CHECK(cbor_idx_any(s, n, idx, &v)); \
    return cbor_as_ ## name(&v, r1, r2); \
  }

CBOR_IDX_1(uint64, uint64_t)
CBOR_IDX_1(uint32, uint32_t)
CBOR_IDX_1(uint16, uint16_t)
CBOR_IDX_1(uint8, uint8_t)
CBOR_IDX_1(int64, int64_t)
CBOR_IDX_1(int32, int32_t)
CBOR_IDX_1(int16, int16_t)
CBOR_IDX_1(int8, int8_t)
CBOR_IDX_1(bool, bool)
CBOR_IDX_0(null)
CBOR_IDX_0(undefined)
CBOR_IDX_1(simple, uint8_t)
#if !defined(CBOR_NO_DECIMAL)
CBOR_IDX_2(decimal, int64_t, int64_t)
#endif
#if !defined(CBOR_NO_RATIONAL)
CBOR_IDX_2(rational, int64_t, uint64_t)
#endif
CBOR_IDX_1(float64, float64_t)
CBOR_IDX_1(datetime, float64_t)
CBOR_IDX_2(tag, cbor_stream_t, uint64_t)
#if !defined(CBOR_NO_ENCODED)
CBOR_IDX_2(encoded, cbor_stream_t, size_t)
#endif
CBOR_IDX_2(text, cbor_stream_t, size_t)
CBOR_IDX_2(bytes, cbor_stream_t, size_t)
CBOR_IDX_2(array, cbor_stream_t, size_t)
CBOR_IDX_2(map, cbor_stream_t, size_t)


cbor_error_t cbor_init(cbor_stream_t* s, uint8_t* b, size_t n) {
  if (s == NULL) return CBOR_ERROR_NULL;
  s->s = b;
  s->b = b;
  s->n = n;
  s->error = CBOR_ERROR_NONE;
  if (b == NULL) return CBOR_ERROR_NULL;
  return CBOR_ERROR_NONE;
}

cbor_error_t cbor_dup(const cbor_stream_t* from, cbor_stream_t* to) {
  if ((from == NULL) || (to == NULL)) return CBOR_ERROR_NULL;
  *to = *from;
  return CBOR_ERROR_NONE;
}

uint8_t* cbor_cursor(const cbor_stream_t* s) {
 if (s == NULL) return NULL;
 return s->b;
}

size_t cbor_write_avail(const cbor_stream_t* s) {
  if (s == NULL) return 0;
  return s->n;
}

size_t cbor_read_avail(const cbor_stream_t*s) {
  if (s == NULL) return 0;
  return s->b - s->s;
}


cbor_error_t cbor_skip(cbor_stream_t* s, size_t n) {
  while (n-- > 0) {
    cbor_value_t v;
    CHECK(cbor_read_any(s, &v));
  }
  return CBOR_ERROR_NONE;
}

cbor_error_t cbor_error(const cbor_stream_t* s) {
  if (s == NULL) return CBOR_ERROR_NULL;
  return s->error;
}

static cbor_error_t write_mt_uint64(cbor_stream_t* s, cbor_type_t mt, uint64_t v) {
  uint8_t* b = s->b;
  if (v < 24) {
    if (s->n < 1) RET_ERROR(s, CBOR_ERROR_END_OF_STREAM);
    b[0] = (uint8_t) (mt << 5) + (uint8_t) v;
    s->b += 1;
    s->n -= 1;
  }
  else if (v < 256) {
    if (s->n < 2) RET_ERROR(s, CBOR_ERROR_END_OF_STREAM);
    b[0] = (uint8_t) (mt << 5) + 24;
    b[1] = (uint8_t) v;
    s->b += 2;
    s->n -= 2;
  }
  else if (v < 65536) {
    if (s->n < 3) RET_ERROR(s, CBOR_ERROR_END_OF_STREAM);
    b[0] = (uint8_t) (mt << 5) + 25;
    b[1] = (v >> 8) & 0xff;
    b[2] = (v >> 0) & 0xff;
    s->b += 3;
    s->n -= 3;
  }
  else if (v < (1LL << 32)) {
    if (s->n < 5) RET_ERROR(s, CBOR_ERROR_END_OF_STREAM);
    b[0] = (uint8_t) (mt << 5) + 26;
    b[1] = (v >> 24) & 0xff;
    b[2] = (v >> 16) & 0xff;
    b[3] = (v >>  8) & 0xff;
    b[4] = (v >>  0) & 0xff;
    s->b += 5;
    s->n -= 5;
  }
  else {
    if (s->n < 9) RET_ERROR(s, CBOR_ERROR_END_OF_STREAM);
    b[0] = (uint8_t) (mt << 5) + 27;
    b[1] = (v >> 56) & 0xff;
    b[2] = (v >> 48) & 0xff;
    b[3] = (v >> 40) & 0xff;
    b[4] = (v >> 32) & 0xff;
    b[5] = (v >> 24) & 0xff;
    b[6] = (v >> 16) & 0xff;
    b[7] = (v >>  8) & 0xff;
    b[8] = (v >>  0) & 0xff;
    s->b += 9;
    s->n -= 9;
  }
  return CBOR_ERROR_NONE;
}

static cbor_error_t write_mt_bytes(cbor_stream_t* s, cbor_type_t mt,
                                const void* b, size_t n) {
  if (s == NULL) return CBOR_ERROR_NULL;
  CHECK(write_mt_uint64(s, mt, n));
  if (s->n < n) RET_ERROR(s, CBOR_ERROR_END_OF_STREAM);
  memmove(s->b, b, n);
  s->b += n;
  s->n -= n;
  return CBOR_ERROR_NONE;
}

static cbor_error_t write_mt_uint8(cbor_stream_t*s, cbor_type_t mt, uint8_t ext) {
  if (s == NULL) return CBOR_ERROR_NULL;
  if (s->n < 1) RET_ERROR(s, CBOR_ERROR_END_OF_STREAM);
  *(s->b)++ = (uint8_t) (mt << 5) + ext;
  s->n--;
  return CBOR_ERROR_NONE;
}

cbor_error_t cbor_write_text(cbor_stream_t* s, const char* cs) {
  if (cs == NULL) return cbor_write_null(s);
  return write_mt_bytes(s, CBOR_TYPE_TEXT, cs, strlen(cs));
}

cbor_error_t cbor_write_textn(cbor_stream_t* s, const char* cs, size_t n) {
  if (cs == NULL) return cbor_write_null(s);
  return write_mt_bytes(s, CBOR_TYPE_TEXT, cs, n);
}

cbor_error_t cbor_write_text_start(cbor_stream_t* s) {
  return write_mt_uint8(s, CBOR_TYPE_TEXT, 31);
}

cbor_error_t cbor_write_end(cbor_stream_t* s) {
  return write_mt_uint8(s, CBOR_TYPE_SIMPLE, 31);
}

cbor_error_t cbor_write_bytes(cbor_stream_t* s, const uint8_t* b, size_t n) {
  return write_mt_bytes(s, CBOR_TYPE_BYTES, b, n);
}

cbor_error_t cbor_write_bytes_start(cbor_stream_t* s) {
  return write_mt_uint8(s, CBOR_TYPE_BYTES, 31);
}

cbor_error_t cbor_write_encoded_cbor(cbor_stream_t* s, const uint8_t* b, size_t n) {
  cbor_error_t e = write_mt_uint64(s, CBOR_TYPE_TAG, 24);
  if (e != CBOR_ERROR_NONE) return e;
  return write_mt_bytes(s, CBOR_TYPE_BYTES, b, n);
}

cbor_error_t cbor_write_self_desc_cbor(cbor_stream_t* s) {
  return write_mt_uint64(s, CBOR_TYPE_TAG, 55799);
}

cbor_error_t cbor_write_simple(cbor_stream_t* s, uint8_t v) {
  return write_mt_uint64(s, CBOR_TYPE_SIMPLE, v);
}

cbor_error_t cbor_write_tag(cbor_stream_t* s, uint64_t tag) {
  return write_mt_uint64(s, CBOR_TYPE_TAG, tag);
}

cbor_error_t cbor_write_array(cbor_stream_t* s, size_t n) {
  return write_mt_uint64(s, CBOR_TYPE_ARRAY, n);
}

cbor_error_t cbor_write_array_start(cbor_stream_t* s) {
  return write_mt_uint8(s, CBOR_TYPE_ARRAY, 31);
}

cbor_error_t cbor_write_map(cbor_stream_t* s, size_t n) {
  return write_mt_uint64(s, CBOR_TYPE_MAP, n);
}

cbor_error_t cbor_write_map_start(cbor_stream_t* s) {
  return write_mt_uint8(s, CBOR_TYPE_MAP, 31);
}

cbor_error_t cbor_write_bool(cbor_stream_t* s, bool b) {
  return write_mt_uint8(s, CBOR_TYPE_SIMPLE, b ? 21: 20);
}

cbor_error_t cbor_write_undefined(cbor_stream_t* s) {
  return write_mt_uint8(s, CBOR_TYPE_SIMPLE, 23);
}

cbor_error_t cbor_write_null(cbor_stream_t* s) {
  return write_mt_uint8(s, CBOR_TYPE_SIMPLE, 22);
}

cbor_error_t cbor_write_uint64(cbor_stream_t* s, uint64_t v) {
  return write_mt_uint64(s, CBOR_TYPE_UINT, v);
}

cbor_error_t cbor_write_int64(cbor_stream_t* s, int64_t v) {
  return  v >= 0 ?  write_mt_uint64(s, CBOR_TYPE_UINT, v) :
                    write_mt_uint64(s, CBOR_TYPE_NINT, ~v);
}

#if !defined(CBOR_NO_FLOAT)
cbor_error_t cbor_write_float16(cbor_stream_t* s, float16_t v) {
  if (s == NULL) return CBOR_ERROR_NULL;

  const uint8_t* b = (const uint8_t*) &v;
  CHECK(write_mt_uint8(s, CBOR_TYPE_SIMPLE, 25));
  if (s->n < 2) RET_ERROR(s, CBOR_ERROR_END_OF_STREAM);
  uint8_t* bb = s->b;
  bb[0] = b[1];
  bb[1] = b[0];
  s->b += 2;
  s->n -= 2;
  return CBOR_ERROR_NONE;
}

static cbor_error_t write_float32(cbor_stream_t* s, float32_t v) {
  if (s == NULL) return CBOR_ERROR_NULL;

  const uint8_t* b = (const uint8_t*) &v;
  CHECK(write_mt_uint8(s, CBOR_TYPE_SIMPLE, 26));
  if (s->n < 4) RET_ERROR(s, CBOR_ERROR_END_OF_STREAM);
  uint8_t* bb = s->b;
  bb[0] = b[3];
  bb[1] = b[2];
  bb[2] = b[1];
  bb[3] = b[0];
  s->b += 4;
  s->n -= 4;
  return CBOR_ERROR_NONE;
}

static cbor_error_t write_float64(cbor_stream_t* s, float64_t v) {
  if (s == NULL) return CBOR_ERROR_NULL;

  const uint8_t* b = (const uint8_t*) &v;
  CHECK(write_mt_uint8(s, CBOR_TYPE_SIMPLE, 27));
  if (s->n < 8) RET_ERROR(s, CBOR_ERROR_END_OF_STREAM);
  uint8_t* bb = s->b;
  bb[0] = b[7];
  bb[1] = b[6];
  bb[2] = b[5];
  bb[3] = b[4];
  bb[4] = b[3];
  bb[5] = b[2];
  bb[6] = b[1];
  bb[7] = b[0];
  s->b += 8;
  s->n -= 8;
  return CBOR_ERROR_NONE;
}

// Good overview of converting float representations:
//   http://www.fox-toolkit.org/ftp/fasthalffloatconversion.pdf
cbor_error_t cbor_write_float32(cbor_stream_t* s, float32_t v) {
  if (isnan(v) || isinf(v)) return cbor_write_float16(s, (float16_t) v);
  float16_t hfv = (float16_t) v;
  return hfv == v ? cbor_write_float16(s, hfv) : write_float32(s, v);
}

cbor_error_t cbor_write_float64(cbor_stream_t* s, float64_t v) {
  if (isnan(v) || isinf(v)) return cbor_write_float16(s, (float16_t) v);

  float32_t fv = (float32_t)v;
  if ((float64_t)fv == v) {
    float16_t hfv = (float16_t) fv;
    return hfv == fv ? cbor_write_float16(s, hfv) : write_float32(s, fv);
  }
  return write_float64(s, v);
}

#endif

#if !defined(CBOR_NO_DATATIME)
cbor_error_t cbor_write_datetime(cbor_stream_t*s, float64_t v) {
  if (s == NULL) return CBOR_ERROR_NULL;
  CHECK(write_mt_uint64(s, CBOR_TYPE_TAG, 1));
  int64_t x = (int64_t) v;
  if ((float64_t) x == v) {
    return cbor_write_int64(s, x);
  }
  else {
    return cbor_write_float64(s, v);
  }
}
#endif

#if !defined(CBOR_NO_FLOAT) && !defined(CBOR_NO_DECIMAL)
cbor_error_t cbor_float642decimal(float64_t v, unsigned digits,
                    int64_t* mant, int64_t* exp) {
  static const float64_t factor[] = { 1.0, 10.0, 100.0, 1000.0, 10000.0, 100000.0, 1000000.0 };

  if (isnan(v) || isinf(v)) {
    return CBOR_ERROR_RANGE;
  }

  if (digits > 6) return CBOR_ERROR_RANGE;
  v = round(v * factor[digits]);
  if ((v > (0x7fffffffffffffffll)) ||
      (-v > (0x7fffffffffffffffll))) {
    return CBOR_ERROR_RANGE;
  }
  *exp =  -((int64_t) digits);
  *mant = llround(v);
  return CBOR_ERROR_NONE;
}
#endif

// Note: RFC 7049 section 2.4.3 specifically calls out:
//   ... there is a quality-of-implementation expectation that the integer
//       representation is used directly.
//
// This will casue a generic dumps/loads to convert the type from "Decimal"
// to "Integer".  This makes testing a bit more complex.  So for the moment
// this implementation ignores this guidance.
cbor_error_t cbor_write_decimal(cbor_stream_t* s, int64_t mant,  int64_t expon) {
  if (s == NULL) return CBOR_ERROR_NULL;

  CHECK(write_mt_uint64(s, CBOR_TYPE_TAG, 4));
  CHECK(cbor_write_array(s, 2));
  CHECK(cbor_write_int64(s, expon));
  return cbor_write_int64(s, mant);
}

// Note: RFC 7049 section 2.4.3 specifically calls out:
//   ... there is a quality-of-implementation expectation that the integer
//       representation is used directly.
//
// For rationals the above is not specifically called out but it would seem
// that similar logic would apply.
//
// This will casue a generic dumps/loads to convert the type from "Decimal"
// to "Integer".  This makes testing a bit more complex.  So for the moment
// this implementation ignores this guidance.
cbor_error_t cbor_write_rational(cbor_stream_t* s, int64_t n, uint64_t d) {
  if (s == NULL) return CBOR_ERROR_NULL;
  CHECK(write_mt_uint64(s, CBOR_TYPE_TAG, 30));
  CHECK(cbor_write_array(s, 2));
  CHECK(cbor_write_int64(s, n));
  return cbor_write_uint64(s, d);
}

typedef struct {
  cbor_stream_t s;
  const char* fmt;
  size_t level;
  va_list args;
} cbor_pack_state_t;


typedef struct {
  cbor_stream_t s;
  const char* fmt;
  size_t level;
  va_list args;
} cbor_unpack_state_t;

static cbor_error_t cbor_pack1(cbor_pack_state_t* state) {
  if (state->level > CBOR_MAX_RECURSION) return CBOR_ERROR_RECURSION;
  switch (*state->fmt++) {
    case '}':
    case ']':
      return CBOR_ERROR_FMT;
    case '{': {
      CHECK(cbor_write_map_start(&state->s));
      state->level += 1;
      while ((*state->fmt != '\0') && (*state->fmt != '}')) {
        // Read the key
        switch (*state->fmt) {
          case '.': {
            state->fmt++;
            size_t n = 0;
            const char* k = state->fmt;
            while ((*state->fmt != '\0') && (*state->fmt != ':')) {
              state->fmt++;
              n++;

            }
            if (n == 0) return CBOR_ERROR_FMT;
            CHECK(cbor_write_textn(&state->s, k, n));
            break;
          }
          case 's':
          case 'i':
            CHECK(cbor_pack1(state));
            break;
          default:
            return CBOR_ERROR_FMT;
            break; // MISRA-C Rule 16.3
        }
        // Read fmt sep
        if (*state->fmt++ != ':') return CBOR_ERROR_FMT;
        CHECK(cbor_pack1(state));

        if (*state->fmt == '}') break;
        if (*state->fmt++ != ',') return CBOR_ERROR_FMT;
      }
      state->level -= 1;
      CHECK(cbor_write_end(&state->s));
      if (*state->fmt++ != '}') return CBOR_ERROR_FMT;
      break;
    }
    case '[': {
      CHECK(cbor_write_array_start(&state->s));
      state->level += 1;
      while ((*state->fmt != '\0') && (*state->fmt != ']')) {
        CHECK(cbor_pack1(state));
        if (*state->fmt == ',') {
          state->fmt++;
        }
      }
      state->level -= 1;
      CHECK(cbor_write_end(&state->s));
      if (*state->fmt++ != ']') return CBOR_ERROR_FMT;
      break;
    }
    case 'I': {
      uint64_t u = va_arg(state->args, uint32_t);
      CHECK(cbor_write_uint64(&state->s, u));
      break;
    }
    case 'Q': {
      uint64_t u = va_arg(state->args, uint64_t);
      CHECK(cbor_write_uint64(&state->s, u));
      break;
    }
    case 'i': {
      int64_t d = va_arg(state->args, int32_t);
      CHECK(cbor_write_int64(&state->s, d));
      break;
    }
    case 'q': {
      int64_t d = va_arg(state->args, int64_t);
      CHECK(cbor_write_int64(&state->s, d));
      break;
    }
    case 's': {
      const char* t = va_arg(state->args, const char*);
      CHECK(cbor_write_text(&state->s, t));
      break;
    }
    case 'b': {
      const uint8_t* b = va_arg(state->args, const uint8_t*);
      size_t n = va_arg(state->args, size_t);
      CHECK(cbor_write_bytes(&state->s, b, n));
      break;
    }
    case '?': {
      int b = va_arg(state->args, int);
      CHECK(cbor_write_bool(&state->s, b));
      break;
    }
#if !defined(CBOR_NO_RATIONAL)
    case 'R': {
      int64_t num = va_arg(state->args, int64_t);
      uint64_t denom = va_arg(state->args, uint64_t);
      CHECK(cbor_write_rational(&state->s, num, denom));
      break;
    }
#endif
#if !defined(CBOR_NO_DECIMAL)
    case 'D': {
      int64_t mant = va_arg(state->args, int64_t);
      int64_t exp = va_arg(state->args, int64_t);
      CHECK(cbor_write_decimal(&state->s, mant, exp));
      break;
    }
#endif
#if !defined(CBOR_NO_FLOAT)
    case 'd': {
      float64_t d = va_arg(state->args, float64_t);
      CHECK(cbor_write_float64(&state->s, d));
      break;
    }
#endif
#if !defined(CBOR_NO_DATETIME)
    case 't': {
      float64_t d = va_arg(state->args, float64_t);
      CHECK(cbor_write_datetime(&state->s, d));
      break;
    }
#endif
    default:
      return CBOR_ERROR_CANT_CONVERT_TYPE;
  }
  return CBOR_ERROR_NONE;
}

cbor_error_t cbor_vpack(cbor_stream_t* s, const char* fmt, va_list args) {
  cbor_pack_state_t state = { .s = *s, .fmt = fmt, .level = 0, .args = args };
  while (*state.fmt != '\0') {
    CHECK(cbor_pack1(&state));
    if (state.s.error != CBOR_ERROR_NONE) {
      LOG_ERROR("cbor_pack \"%s\" offset: %u error: {enum:cbor_error_t}%d", fmt, (unsigned) (state.fmt - fmt), state.s.error);
      *s = state.s;
      return state.s.error;
    }
  }
  *s = state.s;
  return CBOR_ERROR_NONE;
}

cbor_error_t cbor_pack(cbor_stream_t* s, const char* fmt, ...) {
  va_list args;
  va_start(args, fmt);
  cbor_error_t e = cbor_vpack(s, fmt, args);
  va_end(args);
  return e;
}

cbor_error_t cbor_unpack_skip(cbor_unpack_state_t* state) {
  switch (*state->fmt++) {
    case '}':
    case ']':
      return CBOR_ERROR_FMT;
    case '{': {
      state->level += 1;
      while ((*state->fmt != '\0') && (*state->fmt != '}')) {
        // Read the key
        switch (*state->fmt) {
          case '.':
            state->fmt++;
            while ((*state->fmt != '\0') && (*state->fmt != ':')) {
              state->fmt++;
            }
            break;
          case 's':
            (void) va_arg(state->args, const char*);
            break;
          case 'i':
            (void) va_arg(state->args, int);
            break;
          default:
            return CBOR_ERROR_FMT;
        }
        // Read fmt sep
        if (*state->fmt++ != ':') return CBOR_ERROR_FMT;
        if (*state->fmt == '?') {
          bool* b = va_arg(state->args, bool*);
          (void) b;
        }
        cbor_unpack_skip(state);

        if (*state->fmt == '}') break;
        if (*state->fmt++ != ',') return CBOR_ERROR_FMT;
      }
      state->level -= 1;
      if (*state->fmt++ != '}') return CBOR_ERROR_FMT;
      break;
    }
    case '[': {
      state->level += 1;
      while ((*state->fmt != '\0') && (*state->fmt != ']')) {
        CHECK(cbor_unpack_skip(state));
      }
      state->level -= 1;
      if (*state->fmt++ != ']') return CBOR_ERROR_FMT;
      break;
    }
    case 'i': {
      (void) va_arg(state->args, int32_t*);
      break;
    }
    case 'I': {
      (void) va_arg(state->args, uint32_t*);
      break;
    }
    case 'q': {
      (void) va_arg(state->args, int64_t*);
      break;
    }
    case 'Q': {
      (void) va_arg(state->args, uint64_t*);
      break;
    }
    case 's': {
      (void) va_arg(state->args, char*);
      (void) va_arg(state->args, size_t);
      break;
    }
    case 'b': {
      (void) va_arg(state->args, uint8_t*);
      (void) va_arg(state->args, size_t);
      break;
    }
    case '?': {
      (void) va_arg(state->args, bool*);
      break;
    }
    case 'R': {
      (void) va_arg(state->args, int64_t*);
      (void) va_arg(state->args, uint64_t*);
      break;
    }
    case 'D': {
      (void) va_arg(state->args, int64_t*);
      (void) va_arg(state->args, int64_t*);
      break;
    }
#if !defined(CBOR_NO_FLOAT)
    case 'd': {
      (void) va_arg(state->args, float64_t*);
      break;
    }
    case 'f': {
      (void) va_arg(state->args, float32_t*);
      break;
    }
    case 'e': {
      (void) va_arg(state->args, float16_t*);
      break;
    }
#endif
#if !defined(CBOR_NO_DATETIME)
    case 't': {
      (void) va_arg(state->args, float64_t*);
      break;
    }
#endif
    default:
      return CBOR_ERROR_FMT;
  }
  return CBOR_ERROR_NONE;
}

static cbor_error_t cbor_get_textn_stream(cbor_stream_t *s,
                               const char* k, size_t key_n, cbor_stream_t* st) {
  cbor_stream_t s2 = *s;
  while (true) {
    cbor_value_t v;
    cbor_error_t e = cbor_read_any(&s2, &v);
    if (e != CBOR_ERROR_NONE) {
      if (e == CBOR_ERROR_END_OF_STREAM) return CBOR_ERROR_KEY_NOT_FOUND;
      return e;
    }
    if (v.type == CBOR_TYPE_TEXT) {
      if (v.value.stream_v.n == key_n) {
        if (cbor_memcmp(k, &(v.value.stream_v.s), key_n) == 0) {
          *st = s2;
          return CBOR_ERROR_NONE;
        }
      }
    }
    CHECK(cbor_read_any(&s2, &v));
  }
  return CBOR_ERROR_KEY_NOT_FOUND;
}

static cbor_error_t cbor_get_text_stream(cbor_stream_t *s,
                               const char* k, cbor_stream_t* st) {
  return cbor_get_textn_stream(s, k, strlen(k), st);
}

static cbor_error_t cbor_get_int_stream(cbor_stream_t *s,
                               int64_t k, cbor_stream_t* st) {
  cbor_stream_t s2 = *s;
  cbor_value_t v;
  while (true) {
    cbor_error_t e = cbor_read_any(&s2, &v);
    if (e != CBOR_ERROR_NONE) {
      if (e == CBOR_ERROR_END_OF_STREAM) return CBOR_ERROR_KEY_NOT_FOUND;
      return e;
    }
    if ((v.type == CBOR_TYPE_UINT) || (v.type == CBOR_TYPE_NINT)) {
      int64_t kk;
      if (cbor_as_int64(&v, &kk) == CBOR_ERROR_NONE) {
        if (kk == k) {
          *st = s2;
          return CBOR_ERROR_NONE;
        }
      }
    }
    CHECK(cbor_read_any(&s2, &v));
  }
  return CBOR_ERROR_KEY_NOT_FOUND;
}

cbor_error_t cbor_unpack1(cbor_unpack_state_t* state) {
  if (state->level > CBOR_MAX_RECURSION) return CBOR_ERROR_RECURSION;
  switch (*state->fmt++) {
    case '}':
    case ']':
      return CBOR_ERROR_FMT;
    case '{': {
      cbor_stream_t map_s;
      size_t map_n;
      CHECK(cbor_read_map(&state->s, &map_s, &map_n));
      cbor_stream_t s = state->s;
      state->level += 1;
      while (*state->fmt != '\0') {
        cbor_error_t e = CBOR_ERROR_NONE;
        const char* k_str;
        size_t k_str_n;
        int64_t k_int;
        // Read key
        switch (*state->fmt) {
          case '.':
            state->fmt++;
            k_str = state->fmt;
            k_str_n = 0;
            while ((*state->fmt != '\0') && (*state->fmt != ':')) {
              k_str_n++;
              state->fmt++;
            }
            if (k_str_n == 0) return CBOR_ERROR_FMT;
            e = cbor_get_textn_stream(&map_s, k_str, k_str_n, &state->s);
            break;
          case 's':
            k_str = va_arg(state->args, const char*);
            e = cbor_get_text_stream(&map_s, k_str, &state->s);
            break;
          case 'i':
            k_int = va_arg(state->args, int);
            e = cbor_get_int_stream(&map_s, k_int, &state->s);
            break;
          default:
            return CBOR_ERROR_FMT;
        }
        if ((e != CBOR_ERROR_NONE) && (e != CBOR_ERROR_KEY_NOT_FOUND)) {
          return e;
        }

        // Read fmt sep
        if (*state->fmt++ != ':') return CBOR_ERROR_FMT;
        bool local_b = true;
        bool* b = &local_b;
        if (*state->fmt == '?') {
          state->fmt++;
          b = va_arg(state->args, bool*);
          *b = e == CBOR_ERROR_NONE;
        }
        if (*b) {
          if (e == CBOR_ERROR_KEY_NOT_FOUND) return e;
          CHECK(cbor_unpack1(state));
        }
        else {
          CHECK(cbor_unpack_skip(state));
        }
        if (*state->fmt == '}') break;
        if (*state->fmt++ != ',') return CBOR_ERROR_FMT;
      }
      state->s = s;
      state->level -= 1;
      if (*state->fmt++ != '}') return CBOR_ERROR_FMT;
      break;
    }
    case '[': {
      cbor_stream_t array_s;
      size_t array_n;
      CHECK(cbor_read_array(&state->s, &array_s, &array_n));
      state->level += 1;
      cbor_stream_t s = state->s;
      state->s = array_s;
      while ((*state->fmt != '\0') && (*state->fmt != ']')) {
        if (array_n-- == 0) return CBOR_ERROR_ARRAY_TOO_LARGE;
        CHECK(cbor_unpack1(state));
        if (*state->fmt == ',') {
          state->fmt++;
        }
      }
      state->level -= 1;
      state->s = s;
      if (*state->fmt++ != ']') return CBOR_ERROR_FMT;
      break;
    }
    case 'i': {
      int32_t* d;
      d = va_arg(state->args, int32_t*);
      CHECK(cbor_read_int32(&state->s, d));
      break;
    }
    case 'I': {
      uint32_t* u;
      u = va_arg(state->args, uint32_t*);
      CHECK(cbor_read_uint32(&state->s, u));
      break;
    }
    case 'q': {
      int64_t* d;
      d = va_arg(state->args, int64_t*);
      CHECK(cbor_read_int64(&state->s, d));
      break;
    }
    case 'Q': {
      uint64_t* u;
      u = va_arg(state->args, uint64_t*);
      CHECK(cbor_read_uint64(&state->s, u));
      break;
    }
    case 's': {
      char* t;
      size_t* n;
      cbor_stream_t s2;
      size_t n2;
      t = va_arg(state->args, char*);
      n = va_arg(state->args, size_t*);
      CHECK(cbor_read_text(&state->s, &s2, &n2));
      *n -= 1;
      if (n2 > *n) {
        *n = n2 + 1;
        return CBOR_ERROR_BUFFER_TOO_SMALL;
      }
      *n = n2;
      memset(t, 0, *n + 1);
      CHECK(cbor_memmove(t, &s2, *n));
      *n = n2 + 1;
      break;
    }
    case 'b': {
      uint8_t* b;
      size_t* n;
      cbor_stream_t s2;
      size_t n2;
      b = va_arg(state->args, uint8_t*);
      n = va_arg(state->args, size_t*);
      CHECK(cbor_read_bytes(&state->s, &s2, &n2));
      if (n2 > *n) {
        *n = n2;
        return CBOR_ERROR_BUFFER_TOO_SMALL;
      }
      *n = n2;
      memset(b, 0, *n);
      CHECK(cbor_memmove(b, &s2, *n));
      break;
    }
    case '+':
    case '?': {
      bool* b;
      b = va_arg(state->args, bool*);
      CHECK(cbor_read_bool(&state->s, b));
      break;
    }
#if !defined(CBOR_NO_RATIONAL)
    case 'R': {
      int64_t* num = va_arg(state->args, int64_t*);
      uint64_t* denom = va_arg(state->args, uint64_t*);
      CHECK(cbor_read_rational(&state->s, num, denom));
      break;
    }
#endif
#if !defined(CBOR_NO_DECIMAL)
    case 'D': {
      int64_t* mant = va_arg(state->args, int64_t*);
      int64_t* expon = va_arg(state->args, int64_t*);
      CHECK(cbor_read_decimal(&state->s, mant, expon));
      break;
    }
#endif
#if !defined(CBOR_NO_FLOAT)
    case 'd': {
      float64_t* d = va_arg(state->args, float64_t*);
      CHECK(cbor_read_float64(&state->s, d));
      break;
    }
    case 'f': {
      float32_t* d = va_arg(state->args, float32_t*);
      CHECK(cbor_read_float32(&state->s, d));
      break;
    }
    case 'e': {
      float16_t* d = va_arg(state->args, float16_t*);
      CHECK(cbor_read_float16(&state->s, d));
      break;
    }
#endif
#if !defined(CBOR_NO_DATETIME)
    case 't': {
      float64_t* d = va_arg(state->args, float64_t*);
      CHECK(cbor_read_datetime(&state->s, d));
      break;
    }
#endif
    case 'v': {
      cbor_value_t v;
      cbor_stream_t* s = va_arg(state->args, cbor_stream_t*);
      *s = state->s;
      CHECK(cbor_read_any(&state->s, &v));
      break;
    }
    default:
      return CBOR_ERROR_FMT;
  }
  return CBOR_ERROR_NONE;
}

cbor_error_t cbor_unpack(cbor_stream_t*s, const char* fmt, ...) {
  cbor_unpack_state_t state = { .s = *s, .fmt = fmt, .level = 0 };
  va_start(state.args, fmt);
  while (*state.fmt != '\0') {
    cbor_error_t e = cbor_unpack1(&state);
    if ((e != CBOR_ERROR_NONE) && (state.s.error == CBOR_ERROR_NONE)) {
      state.s.error = e;
    }
    if (state.s.error != CBOR_ERROR_NONE) {
      LOG_ERROR("cbor_unpack \"%s\" offset: %u error: {enum:cbor_error_t}%d", fmt, (unsigned) (state.fmt - fmt), state.s.error);
      return state.s.error;
    }
  }
  return CBOR_ERROR_NONE;
}
