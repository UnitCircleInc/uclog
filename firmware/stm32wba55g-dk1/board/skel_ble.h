// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0
//
// Stands in for the BLE_p2pServer_Simplest example's skel_ble.h, which ST's platform files
// (host_stack_if.c, ll_sys_if.c) include: the two "processing needed" callbacks, in ble.c.

#pragma once

void ll_process(void);            // the link layer has background work
void Ble_HostStack_Process(void); // the BLE host stack has background work
