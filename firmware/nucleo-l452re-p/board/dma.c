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


#include <stdint.h>
#include <stdbool.h>
#include <dma.h>
#include <stm32l4xx_ll_bus.h>
#include <stm32l4xx_ll_dma.h>

#define DECLARE_DMA_HANDLER(ch_)                     \
void dma_ch##ch_##_handler(dma_t* dma) {             \
  if(LL_DMA_IsActiveFlag_TC##ch_(dma->dma)) {        \
    LL_DMA_ClearFlag_GI##ch_(dma->dma);              \
    if (dma->complete) dma->complete(dma->ctx);      \
  }                                                  \
  else if(LL_DMA_IsActiveFlag_TE##ch_(dma->dma)) {   \
    LL_DMA_ClearFlag_GI##ch_(dma->dma);              \
    if (dma->error) dma->error(dma->ctx);            \
  }                                                  \
}

DECLARE_DMA_HANDLER(1)
DECLARE_DMA_HANDLER(2)
DECLARE_DMA_HANDLER(3)
DECLARE_DMA_HANDLER(4)
DECLARE_DMA_HANDLER(5)
DECLARE_DMA_HANDLER(6)
DECLARE_DMA_HANDLER(7)


void dma_init(dma_t* dma, void (*complete)(void* ctx), void (*error)(void* ctx),
              void* ctx) {
  switch ((uint32_t) dma->dma) {
    case DMA1_BASE: LL_AHB1_GRP1_EnableClock(LL_AHB1_GRP1_PERIPH_DMA1); break;
    case DMA2_BASE: LL_AHB1_GRP1_EnableClock(LL_AHB1_GRP1_PERIPH_DMA2); break;
    default: for(;;) __NOP();
  }

  NVIC_SetPriority(dma->irq, 0);
  NVIC_EnableIRQ(dma->irq);
  LL_DMA_ConfigTransfer(dma->dma, dma->ch, dma->trans);
  LL_DMA_SetPeriphAddress(dma->dma, dma->ch, dma->addr);
  LL_DMA_SetPeriphRequest(dma->dma, dma->ch, dma->req);
  LL_DMA_EnableIT_TC(dma->dma, dma->ch);
  LL_DMA_EnableIT_TE(dma->dma, dma->ch);

  dma->complete = complete;
  dma->error = error;
  dma->ctx = ctx;
}

void dma_stop(dma_t* dma) {
  LL_DMA_DisableChannel(dma->dma, dma->ch);
}

void dma_start(dma_t* dma, const void* b, size_t n) {
  LL_DMA_SetMemoryAddress(dma->dma, dma->ch, (uint32_t) b);
  LL_DMA_SetDataLength(dma->dma, dma->ch, n);
  LL_DMA_EnableChannel(dma->dma, dma->ch);
}

void dma_start_dir(dma_t* dma, const void* b, size_t n, bool m2p) {
  LL_DMA_SetDataTransferDirection(dma->dma, dma->ch,
   m2p ? LL_DMA_DIRECTION_MEMORY_TO_PERIPH : LL_DMA_DIRECTION_PERIPH_TO_MEMORY);
  LL_DMA_SetMemoryAddress(dma->dma, dma->ch, (uint32_t) b);
  LL_DMA_SetDataLength(dma->dma, dma->ch, n);
  LL_DMA_EnableChannel(dma->dma, dma->ch);
}

size_t dma_get_length(dma_t* dma) {
  return LL_DMA_GetDataLength(dma->dma, dma->ch);
}
