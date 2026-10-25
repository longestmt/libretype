// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
// Test-only independent Clang consumer. Never linked into the application.
#import <Foundation/Foundation.h>
#import <CoreGraphics/CoreGraphics.h>
#include <Block.h>
#include <pthread.h>

void *lt_copy(void *block) { return Block_copy(block); }
void lt_release(void *block) { Block_release(block); }

struct Work { void *block; void *first; void *second; int image; };
static void *run(void *argument) {
    struct Work *work = argument;
    if (work->image) {
        void (^callback)(CGImageRef, NSError *) = work->block;
        callback((CGImageRef)work->first, (NSError *)work->second);
    } else {
        void (^callback)(id, id) = work->block;
        callback((id)work->first, (id)work->second);
    }
    return NULL;
}
static int invoke(void *block, void *first, void *second, int image) {
    struct Work work = { block, first, second, image };
    pthread_t thread;
    int status = pthread_create(&thread, NULL, run, &work);
    return status ? status : pthread_join(thread, NULL);
}
int lt_objects_on_worker(void *block, void *first, void *second) {
    return invoke(block, first, second, 0);
}
int lt_image_on_worker(void *block, void *first, void *second) {
    return invoke(block, first, second, 1);
}
