/*
For licensing see accompanying LICENSE file.
Copyright (C) 2026 Apple Inc. All Rights Reserved.
*/

#include "FoundationModels.h"
#include <stdio.h>

typedef struct
{
  size_t lastLength;
  bool isResponding;
} GenerationContext;

void responseCallback(int status, const char *content, size_t length, void *userInfo)
{
  GenerationContext *context = (GenerationContext *)userInfo;
  if (status != 0)
  {
    printf("Failed to respond (error: %d)\n", status);
    context->isResponding = false;
    return;
  }
  if (content)
  {
    printf("%s", &content[context->lastLength]);
    fflush(stdout); // Don't buffer while streaming.
    context->lastLength = length;
  }
  else
  {
    printf("\n✅\n");
    context->isResponding = false;
  }
}

int main()
{
  FMSystemLanguageModelRef model = FMSystemLanguageModelGetDefault();
  FMSystemLanguageModelUnavailableReason unavailableReason = FMSystemLanguageModelUnavailableReasonUnknown;
  bool isAvailable = FMSystemLanguageModelIsAvailable(model, &unavailableReason);
  if (isAvailable)
  {
    printf("Model is available\n");
  }
  else
  {
    printf("Model is unavailable (reason: %d)\n", (int)unavailableReason);
  }

  FMLanguageModelSessionRef session = FMLanguageModelSessionCreateFromSystemLanguageModel(model, /*instructions*/ "Your responses MUST be full of sarcasm.", NULL, 0);
  FMLanguageModelSessionResponseStreamRef stream = FMLanguageModelSessionStreamResponse(session, "What programming language is better, Swift or C?", NULL);
  GenerationContext context;
  context.lastLength = 0;
  context.isResponding = true;
  FMLanguageModelSessionResponseStreamIterate(stream, &context, responseCallback);
  while (context.isResponding)
    ;
  FMRelease(stream);
  FMRelease(session);
  FMRelease(model);
  return 0;
}
