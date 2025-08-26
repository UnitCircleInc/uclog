// © 2023 Unit Circle Inc.
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

#pragma once

#include <stdlib.h>

typedef struct {
  void*  b;
  size_t n;
  size_t read;
  size_t write;
} cb_t;

#define CB_INIT(b_) { .b = b_, .n = sizeof(b_), .read = 0, .write = 0 }

void cb_init(cb_t* cb, void* b, size_t n);
void cb_reset(cb_t* cb);

size_t cb_read_avail(const cb_t *cb);
void cb_read(cb_t* cb, void* b, size_t n);

size_t cb_peek_avail(const cb_t *cb);
const void* cb_peek(const cb_t* cb);
void cb_skip(cb_t* cb, size_t n);

size_t cb_write_avail(const cb_t* cb);
void cb_write(cb_t* cb, const void* in, size_t n);

size_t cb_space_avail(const cb_t *cb);
const void* cb_space(const cb_t* cb);
void cb_commit(cb_t* cb, size_t n);

