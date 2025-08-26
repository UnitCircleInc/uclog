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

// Encoder decoder support is geared to what the following languages can
// represent: C, python, JavaScript.
// It is recommended that:
// - Map keys be limited to integers or text strings.
// - Simultaneous use of keys that integers and text strings that are the same
//   "value" be avoided.
//   e.g. 1 and "1".
// - Use text keys that avoid use of null characters.
//
// Encoder details:
// - Integers as small as possible
// - Lengths for major types 2-5 are as short as possible
// - Keys are not sorted by the library.  Thus the encoder is not-canonical.
//   This can be done in the application
// - Application can choose to use either definite-length or indefinate-length
//   items when encoding.
// - Application can choose to limit keys for maps to text if desired.
// - cbor_write_text assumes input is null terminated UTF-8.
//   Use cbor_write_textn to write strings that have nulls.
// - Library does not check that map keys are unique.
// - Datatime values are encoded epoch-based date/time Tag 1.
// - Decimal fraction using Tag 4 (mantissa as integer)
// - Encoded CBOR data time (Tag 24)
// - Rational using Tag 30 (num/denom as integers).
// - Self-describe CBOR (Tag 55799)
//
// Decoder details:
// - Decoder is not "strict" in the sense that it:
//   - Does not reject maps with duplciate keys.
// - When looking up values in maps that have duplicate keys, the first
//   value that has the matching key will be returned.
// - Will accept integers that are not as small as possible.
// - Will accept lengths for major types 2-5 that are not as short as possible.
// - Will accept any number representation for a numeric value.
// - cbor_get_<xxx> functions assume keys are null terminated UTF-8.
// - Supported tags
//   - Datetime standard date/time string (Tag 0)
//   - Datetime epoch-based date/time (Tag 1)
//   - Decimal fraction (mantissa as integer) (Tag 4)
//   - Encoded CBOR data time (Tag 24)
//   - Rational (num/denom as integers) (Tag 30)
//   - Self-describe CBOR (Tag 55799)

#pragma once
#include <stdlib.h>
#include <stddef.h>
#include <stdint.h>
#include <stdbool.h>
#include <stdarg.h>

// NOTE:
//  It would seem to be "better" to use the fixed width types _Float16,
//  _Float32, and _Float64.   However these are distinct types from __fp16,
//  float, and double.  Specifically casting float32_t arguments to
//  float64_t arguments when float64_t is _Float64 causes compilation errors.
//  _Float16 seems better than __fp16, caluculations in __fp16 are done in float
//  whereas calculations in _Float16 are done in _Float16.
//  There are also compatibility issues with zephyr/include/zephyr/dsp/types.h
//  in rel 3.x.
//  See: https://releases.llvm.org/16.0.0/tools/clang/docs/LanguageExtensions.html#:~:text=__fp16
#if defined(__x86_64_) || defined(__i386)
typedef int16_t float16_t;
#else
typedef __fp16 float16_t;
#endif
typedef float float32_t;
typedef double float64_t;

typedef enum {
  // These map to the standard known types - must be in same order as spec
  CBOR_TYPE_UINT,
  CBOR_TYPE_NINT,
  CBOR_TYPE_BYTES,
  CBOR_TYPE_TEXT,
  CBOR_TYPE_ARRAY,
  CBOR_TYPE_MAP,
  CBOR_TYPE_TAG,     // Returned for unknown tag items
  CBOR_TYPE_SIMPLE,  // Returened for unknown simple items

  // The following are "known" tag types
  CBOR_TYPE_DECIMAL,
  CBOR_TYPE_RATIONAL,
  CBOR_TYPE_DATETIME,
  CBOR_TYPE_ENCODED,

  // The following are "known" simple values
  CBOR_TYPE_BOOL,
  CBOR_TYPE_NULL,
  CBOR_TYPE_UNDEFINED,
  CBOR_TYPE_FLOAT16,
  CBOR_TYPE_FLOAT32,
  CBOR_TYPE_FLOAT64,
} cbor_type_t;

typedef enum {
  // Errors returned by cbor_ready_any and any of the convenience functions
  CBOR_ERROR_NONE,
  CBOR_ERROR_END_OF_STREAM,
  CBOR_ERROR_INVALID_AI,
  CBOR_ERROR_INDEF_MISMATCH,
  CBOR_ERROR_INDEF_NESTING,
  CBOR_ERROR_INVALID_UTF8,
  CBOR_ERROR_BUFFER_TOO_SMALL,
  CBOR_ERROR_BAD_TYPE,
  CBOR_ERROR_RECURSION,
  CBOR_ERROR_MAP_LENGTH,
  CBOR_ERROR_BAD_SIMPLE_VALUE,
  CBOR_ERROR_UNEXPECTED_BREAK,
  CBOR_ERROR_NULL,
  CBOR_ERROR_ITEM_TOO_LONG,
  CBOR_ERROR_INTERNAL_1,
  CBOR_ERROR_INTERNAL_2,

  // Values returned by cbor_as_xxx
  CBOR_ERROR_RANGE,
  CBOR_ERROR_KEY_NOT_FOUND,
  CBOR_ERROR_BAD_DATETIME,
  CBOR_ERROR_BAD_FLOAT64,
  CBOR_ERROR_BAD_DECIMAL,
  CBOR_ERROR_BAD_RATIONAL,
  CBOR_ERROR_BAD_ENCODED,
  CBOR_ERROR_CANT_CONVERT_TYPE,
  CBOR_ERROR_IDX_TOO_BIG,

  CBOR_ERROR_FMT,
  CBOR_ERROR_ARRAY_TOO_LARGE,

  CBOR_ERROR_MARKER = 256, // used to force type to be more than 8 bits
} cbor_error_t;

typedef struct cbor_stream_s {
  uint8_t* s;
  uint8_t* b;
  size_t   n;
  cbor_error_t error;
} cbor_stream_t;

// Structure for holding cbor values
typedef struct {
  cbor_type_t type;
  union {
    uint64_t uint_v;
    uint64_t nint_v;
    bool     bool_v;
    float16_t float16_v;
    float32_t   float32_v;
    float64_t   float64_v;
    uint8_t  simple_v;
    struct {
      cbor_stream_t s;
      size_t n;
    } stream_v;
    struct {
      cbor_stream_t s;
      uint64_t tag;
    } tag_v;
#if !defined(CBOR_NO_DECIMAL)
    struct {
      int64_t mant;
      int64_t expon;
    } decimal_v;
#endif
#if !defined(CBOR_NO_RATIONAL)
    struct {
      int64_t n;
      uint64_t d;
    } rational_v;
#endif
    float64_t datetime_v;
  } value;
} cbor_value_t;


// Initialization and misc functions
cbor_error_t cbor_init(cbor_stream_t* s, uint8_t* b, size_t n);
cbor_error_t cbor_dup(const cbor_stream_t* from, cbor_stream_t* to);
uint8_t* cbor_cursor(const cbor_stream_t* s);
size_t cbor_write_avail(const cbor_stream_t* s);
size_t cbor_read_avail(const cbor_stream_t* s);
cbor_error_t cbor_error(const cbor_stream_t* s);

// Reads the next value in the stream
cbor_error_t cbor_read_any(cbor_stream_t* s, cbor_value_t* v);

// Gets an entry from a map using string key
cbor_error_t cbor_get_any(cbor_stream_t *s, size_t n, const char* key, cbor_value_t* v);

// Gets an entry from an array by index
cbor_error_t cbor_idx_any(const cbor_stream_t *s, size_t n, size_t idx, cbor_value_t* v);

// Converts/casts a read value to a useful "C" type
cbor_error_t cbor_as_uint64(const cbor_value_t* v, uint64_t *r);
cbor_error_t cbor_as_int64(const cbor_value_t* v, int64_t* r);
cbor_error_t cbor_as_bool(const cbor_value_t* v, bool* r);
cbor_error_t cbor_as_null(const cbor_value_t* v);
cbor_error_t cbor_as_undefined(const cbor_value_t* v);
cbor_error_t cbor_as_simple(const cbor_value_t* v, uint8_t* r);
cbor_error_t cbor_as_float64(const cbor_value_t* v, float64_t* d);
cbor_error_t cbor_as_float32(const cbor_value_t* v, float32_t* d);
cbor_error_t cbor_as_float16(const cbor_value_t* v, float16_t* d);
cbor_error_t cbor_as_decimal(const cbor_value_t* v, int64_t* mant, int64_t* expon);
cbor_error_t cbor_as_rational(const cbor_value_t* v, int64_t* n, uint64_t* d);
cbor_error_t cbor_as_datetime(const cbor_value_t*v, float64_t* datetime);

// These return a new stream and the size of the item.
// Size for tag, encoded, text and bytes values is the "expanded" size in bytes
//   i.e. the number of bytes that would be required to copy out
//        the tag, encoded, text or bytes representation with no CBOR encoding
// Size for array or map is in number of entires (indices or key/value pairs)
cbor_error_t cbor_as_tag(const cbor_value_t* v, cbor_stream_t* t, uint64_t* tag);
cbor_error_t cbor_as_encoded(const cbor_value_t* v, cbor_stream_t* t, size_t* n);
cbor_error_t cbor_as_text(const cbor_value_t* v, cbor_stream_t* t, size_t* n);
cbor_error_t cbor_as_bytes(const cbor_value_t* v, cbor_stream_t* t, size_t* n);
cbor_error_t cbor_as_array(const cbor_value_t* v, cbor_stream_t* t, size_t* n);
cbor_error_t cbor_as_map(const cbor_value_t* v, cbor_stream_t* t, size_t* n);

// Functions for converting encoded, text or bytes to linear memory or
// comparing to linear memory
cbor_error_t cbor_memmove(void* b, cbor_stream_t* s, size_t n);
int cbor_memcmp(const void* b, cbor_stream_t* s, size_t n);
int cbor_strcmp(const char* t, cbor_stream_t* s);

// Create a set of convenenance functions for:
// * reading values from a stream - cbor_read_XXX(...)
// * reading values from a map - cbor_get_XXX(...)
// * reading values from an array - cbor_idx_XXX(...)
#define CONV_0(name) \
cbor_error_t cbor_read_ ## name(cbor_stream_t* s); \
cbor_error_t cbor_get_ ## name(cbor_stream_t* s, size_t n, const char* k); \
cbor_error_t cbor_idx_ ## name(const cbor_stream_t* s, size_t n, size_t idx);

#define CONV_1(name, type) \
cbor_error_t cbor_read_ ## name(cbor_stream_t* s, type* u);\
cbor_error_t cbor_get_ ## name(cbor_stream_t* s, size_t n, const char* k, type* u); \
cbor_error_t cbor_idx_ ## name(const cbor_stream_t* s, size_t n, size_t idx, type* u);

#define CONV_2(name, type1, type2) \
cbor_error_t cbor_read_ ## name(cbor_stream_t* s, type1* r1, type2* r2); \
cbor_error_t cbor_get_ ## name(cbor_stream_t* s, size_t n, const char* k, type1* r1, type2* r2); \
cbor_error_t cbor_idx_ ## name(const cbor_stream_t* s, size_t n, size_t idx, type1* r1, type2* r2);

CONV_1(uint64, uint64_t)
CONV_1(uint32, uint32_t)
CONV_1(uint16, uint16_t)
CONV_1(uint8, uint8_t)
CONV_1(int64, int64_t)
CONV_1(int32, int32_t)
CONV_1(int16, int16_t)
CONV_1(int8, int8_t)
CONV_1(bool, bool)
CONV_0(null)
CONV_0(undefined)
CONV_1(simple, uint8_t)
#if !defined(CBOR_NO_DECIMAL)
CONV_2(decimal, int64_t, int64_t)
#endif
#if !defined(CBOR_NO_RATIONAL)
CONV_2(rational, int64_t, uint64_t)
#endif
CONV_1(float64, float64_t)
CONV_1(float32, float32_t)
CONV_1(float16, float16_t)
CONV_1(datetime, float64_t)
CONV_2(tag, cbor_stream_t, uint64_t)
CONV_2(encoded, cbor_stream_t, size_t)
CONV_2(text, cbor_stream_t, size_t)
CONV_2(bytes, cbor_stream_t, size_t)
CONV_2(array, cbor_stream_t, size_t)
CONV_2(map, cbor_stream_t, size_t)


cbor_error_t cbor_write_text(cbor_stream_t* s, const char* cs);
cbor_error_t cbor_write_textn(cbor_stream_t* s, const char* cs, size_t n);
cbor_error_t cbor_write_text_start(cbor_stream_t* s);
cbor_error_t cbor_write_end(cbor_stream_t* s);

cbor_error_t cbor_write_bytes(cbor_stream_t* s, const uint8_t* b, size_t n);
cbor_error_t cbor_write_bytes_start(cbor_stream_t* s);

cbor_error_t cbor_write_self_desc_cbor(cbor_stream_t* s);
cbor_error_t cbor_write_encoded_cbor(cbor_stream_t* s, const uint8_t* b, size_t n);

cbor_error_t cbor_write_simple(cbor_stream_t*s, uint8_t value);
cbor_error_t cbor_write_tag(cbor_stream_t*s, uint64_t value);

cbor_error_t cbor_write_array(cbor_stream_t* s, size_t n);
cbor_error_t cbor_write_array_start(cbor_stream_t* s);

cbor_error_t cbor_write_map(cbor_stream_t* s, size_t n);
cbor_error_t cbor_write_map_start(cbor_stream_t* s);

cbor_error_t cbor_write_bool(cbor_stream_t* s, bool v);
cbor_error_t cbor_write_undefined(cbor_stream_t* s);
cbor_error_t cbor_write_null(cbor_stream_t* s);
cbor_error_t cbor_write_uint64(cbor_stream_t* s, uint64_t v);
cbor_error_t cbor_write_int64(cbor_stream_t* s, int64_t v);
cbor_error_t cbor_write_float16(cbor_stream_t* s, float16_t v);
cbor_error_t cbor_write_float32(cbor_stream_t* s, float32_t v);
cbor_error_t cbor_write_float64(cbor_stream_t* s, float64_t v);
cbor_error_t cbor_write_datetime(cbor_stream_t* s, float64_t v);
cbor_error_t cbor_write_decimal(cbor_stream_t* s, int64_t mant, int64_t expon);
cbor_error_t cbor_write_rational(cbor_stream_t* s, int64_t n, uint64_t d);

// pack C values to structured CBOR stream
//   {<key>:<value>, ...} - map
//     <key> can be one of:
//       .<string> - map const key value string terminates at first ':'
//       s - a text value read a const char* parameter
//       i - an interger value read as a int parameter
//    <value> can be any fmt value
//  [<value>, ...] - array
//    <value> can be any fmt value
//  i - 32 bit signed integer value read as int parameter
//  I - 32 bit unsigned integer value read as unsigned parameter
//  q - 64 bit signed integer value read as int64_t parameter
//  Q - 64 bit signed integer value read as uint64_t parameter
//  ? - bool value read as bool parameter
//  s - a C null terminated utf8 string read as const char* parameter
//  S - a utf8 string read as const char* and size_t parameters
//  b - a list of bytes read as const utf_8* and size_t parameter
//  R - a rational read as (num,denom) as int64_t and uint64_t parameters
//  D - a decimal read as (mant,exp) as int64_t and int64_t parameters
//  d - a float64 read as float64_t paramter
//  t - a timestamp read as float64_t parameter
cbor_error_t cbor_pack(cbor_stream_t*s, const char*fmt, ...);
cbor_error_t cbor_vpack(cbor_stream_t*s, const char*fmt, va_list args);

// unpack CBOR structured stream to C values
//   {<key>:<value>, ...} - map
//   {<key>:?<value>, ...} - map allows for missing <key>
//                          expects bool* parameter which indicates if key is
//                          present.
//     <key> can be one of:
//       .<string> - map const key value string terminates at first ':'
//       s - a text value read a const char* parameter
//       i - an interger value read as a int parameter
//    <value> can be any fmt value
//  [<value>, ...] - array
//    <value> can be any fmt value
//  i - 32 bit signed integer value read as int* parameter
//  I - 32 bit unsigned integer value read as unsigned* parameter
//  q - 64 bit signed integer value read as int64_t* parameter
//  Q - 64 bit signed integer value read as uint64_t* parameter
//  ? - bool value read as bool* parameter
//  s - a utf8 string read as char* and size_t* parameters
//      size_t is read as max length and written as actual length
//      adds a null to the end of the actual string
//      max length needs to include space for null
//      actual length includes added null
//  b - a list of bytes read as utf8* and size_t* parameter
//      size_t is read as max length and written as actual length
//  R - a rational read as (num,denom) as int64_t* and uint64_t* parameters
//  D - a decimal read as (mant,exp) as int64_t* and int64_t* parameters
//  d - a float64 read as float64_t* paramter
//  f - a float32 read as float32_t* paramter
//  e - a float16 read as float16_t* paramter
//  t - a timestamp read as double* parameter
//  v - a cbor value read as cbor_stream_t* paramater
cbor_error_t cbor_unpack(cbor_stream_t*s, const char*fmt, ...);
