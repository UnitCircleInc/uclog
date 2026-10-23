// © 2022 Unit Circle Inc.
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
#include <stdint.h>
#include <stdbool.h>
#include <stm32l4xx_ll_dma.h>

typedef struct {
  DMA_TypeDef* dma;
  uint32_t ch;
  uint32_t req;
  uint32_t irq;
  uint32_t addr;
  uint32_t trans;
  void     (*complete)(void* ctx);
  void     (*error)(void* ctx);
  void*    ctx;
} dma_t;

void dma_ch1_handler(dma_t* dma);
void dma_ch2_handler(dma_t* dma);
void dma_ch3_handler(dma_t* dma);
void dma_ch4_handler(dma_t* dma);
void dma_ch5_handler(dma_t* dma);
void dma_ch6_handler(dma_t* dma);
void dma_ch7_handler(dma_t* dma);

void dma_init(dma_t* dma, void (*complete)(void* ctx), void (*error)(void* ctx),
              void* ctx);
void dma_stop(dma_t* dma);
void dma_start(dma_t* dma, const void* b, size_t n);
void dma_start_dir(dma_t* dma, const void* b, size_t n, bool m2p);
size_t dma_get_length(dma_t* dma);
