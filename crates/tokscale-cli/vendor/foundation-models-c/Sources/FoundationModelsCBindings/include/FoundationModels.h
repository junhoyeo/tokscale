/*
For licensing see accompanying LICENSE file.
Copyright (C) 2026 Apple Inc. All Rights Reserved.
*/

#ifndef FoundationModels_h
#define FoundationModels_h

#include <stdbool.h>
#include <stdint.h>
#include <stddef.h>

typedef const void *_Nonnull FMTaskRef;
typedef const void *FMSystemLanguageModelRef;
typedef const void *FMLanguageModelSessionRef;
typedef const void *FMLanguageModelSessionResponseStreamRef;
typedef const void *FMGenerationSchemaRef;
typedef const void *FMGeneratedContentRef;
typedef const void *FMGenerationSchemaPropertyRef;
typedef const void *FMBridgedToolRef;

// Callbacks
typedef void (*_Nonnull FMLanguageModelSessionResponseCallback)(int status, const char *_Nullable content, size_t length, void *_Nullable userInfo) __attribute__((swift_attr("@Sendable")));
typedef void (*_Nonnull FMLanguageModelSessionStructuredResponseCallback)(int status, FMGeneratedContentRef _Nullable content, void *_Nullable userInfo) __attribute__((swift_attr("@Sendable")));

// MARK: - SystemLanguageModel

// Availability enum
typedef enum
{
  FMSystemLanguageModelUnavailableReasonAppleIntelligenceNotEnabled = 0,
  FMSystemLanguageModelUnavailableReasonDeviceNotEligible = 1,
  FMSystemLanguageModelUnavailableReasonModelNotReady = 2,
  FMSystemLanguageModelUnavailableReasonUnknown = 0xFF
} FMSystemLanguageModelUnavailableReason;

// Use case enum
typedef enum
{
  FMSystemLanguageModelUseCaseGeneral = 0,
  FMSystemLanguageModelUseCaseContentTagging = 1
} FMSystemLanguageModelUseCase;

// Guardrails enum
typedef enum
{
  FMSystemLanguageModelGuardrailsDefault = 0,
  FMSystemLanguageModelGuardrailsPermissiveContentTransformations = 1,
} FMSystemLanguageModelGuardrails;

FMSystemLanguageModelRef _Nonnull FMSystemLanguageModelGetDefault();
FMSystemLanguageModelRef _Nonnull FMSystemLanguageModelCreate(FMSystemLanguageModelUseCase useCase, FMSystemLanguageModelGuardrails guardrails);
bool FMSystemLanguageModelIsAvailable(FMSystemLanguageModelRef _Nonnull ref, FMSystemLanguageModelUnavailableReason *_Nullable unavailableReason);
FMLanguageModelSessionRef _Nonnull FMLanguageModelSessionCreateDefault();
FMLanguageModelSessionRef _Nonnull FMLanguageModelSessionCreateFromSystemLanguageModel(FMSystemLanguageModelRef _Nullable model, const char *_Nullable instructions, FMBridgedToolRef _Nullable *_Nullable tools, int toolCount);

// MARK: - LanguageModelSession

// Prompt construction

typedef const void *_Nonnull FMComposedPrompt;

FMComposedPrompt _Nonnull FMComposedPromptInitialize();

typedef enum {
    FMComposedPromptAddImageErrorNone,
    FMComposedPromptAddImageErrorUnsupported,
    FMComposedPromptAddImageErrorUnknown
} FMComposedPromptAddImageError;

void FMComposedPromptAddText(FMComposedPrompt _Nonnull composedPrompt, const char *_Nonnull text);
bool FMComposedPromptAddImage(FMComposedPrompt _Nonnull composedPrompt, const char *_Nonnull imagePath, FMComposedPromptAddImageError * _Nullable error);
bool FMComposedPromptAddIdentifiedImage(FMComposedPrompt _Nonnull composedPrompt, const char *_Nonnull imagePath, const char *_Nonnull imageIdentifier, FMComposedPromptAddImageError * _Nullable error);
bool FMComposedPromptAddAttachment(FMComposedPrompt _Nonnull composedPrompt, const char *_Nonnull imagePath, const char *_Nullable label, FMComposedPromptAddImageError * _Nullable error);

// Response functions

FMLanguageModelSessionRef _Nonnull FMLanguageModelSessionCreateFromTranscript(FMLanguageModelSessionRef _Nonnull transcriptSession, FMSystemLanguageModelRef _Nullable model, FMBridgedToolRef _Nullable *_Nullable tools, int toolCount);
bool FMLanguageModelSessionIsResponding(FMLanguageModelSessionRef _Nonnull session);
void FMLanguageModelSessionReset(FMLanguageModelSessionRef _Nonnull session);
FMTaskRef FMLanguageModelSessionRespond(FMLanguageModelSessionRef _Nonnull session, FMComposedPrompt _Nonnull composedPrompt, const char *_Nullable optionsJSON, void *_Nullable userInfo, FMLanguageModelSessionResponseCallback callback);
FMLanguageModelSessionResponseStreamRef _Nonnull FMLanguageModelSessionStreamResponse(FMLanguageModelSessionRef _Nonnull session, FMComposedPrompt _Nonnull composedPrompt, const char *_Nullable optionsJSON);
void FMLanguageModelSessionResponseStreamIterate(FMLanguageModelSessionResponseStreamRef _Nonnull stream, void *_Nullable userInfo, FMLanguageModelSessionResponseCallback callback);

// Transcript functions
FMLanguageModelSessionRef _Nullable FMTranscriptCreateFromJSONString(const char *_Nonnull jsonString, int *_Nullable outErrorCode, char *_Nullable *_Nullable outErrorDescription);
char *_Nullable FMLanguageModelSessionGetTranscriptJSONString(FMLanguageModelSessionRef _Nonnull session, int *_Nullable outErrorCode, char *_Nullable *_Nullable outErrorDescription);

// GenerationSchema functions
FMGenerationSchemaRef _Nonnull FMGenerationSchemaCreate(const char *_Nonnull name, const char *_Nullable description);
FMGenerationSchemaPropertyRef _Nonnull FMGenerationSchemaPropertyCreate(const char *_Nonnull name, const char *_Nullable description, const char *_Nonnull typeName, bool isOptional);
void FMGenerationSchemaPropertyAddAnyOfGuide(FMGenerationSchemaPropertyRef _Nonnull property, const char *_Nonnull *_Nonnull anyOf, int choiceCount, bool wrapped);
void FMGenerationSchemaPropertyAddCountGuide(FMGenerationSchemaPropertyRef _Nonnull property, int count, bool wrapped);
void FMGenerationSchemaPropertyAddMaximumGuide(FMGenerationSchemaPropertyRef _Nonnull property, double maximum, bool wrapped);
void FMGenerationSchemaPropertyAddMinimumGuide(FMGenerationSchemaPropertyRef _Nonnull property, double minimum, bool wrapped);
void FMGenerationSchemaPropertyAddMinItemsGuide(FMGenerationSchemaPropertyRef _Nonnull property, int minItems);
void FMGenerationSchemaPropertyAddMaxItemsGuide(FMGenerationSchemaPropertyRef _Nonnull property, int maxItems);
void FMGenerationSchemaPropertyAddRangeGuide(FMGenerationSchemaPropertyRef _Nonnull property, double minValue, double maxValue, bool wrapped);
void FMGenerationSchemaPropertyAddRegex(FMGenerationSchemaPropertyRef _Nonnull property, const char *_Nonnull pattern, bool wrapped);
void FMGenerationSchemaAddProperty(FMGenerationSchemaRef _Nonnull schema, FMGenerationSchemaPropertyRef _Nonnull property);
void FMGenerationSchemaAddReferenceSchema(FMGenerationSchemaRef _Nonnull schema, FMGenerationSchemaRef _Nonnull referenceSchema);
char *_Nullable FMGenerationSchemaGetJSONString(FMGenerationSchemaRef _Nonnull schema, int *_Nullable outErrorCode, char *_Nullable *_Nullable outErrorDescription);

// MARK: - GeneratedContent

FMGeneratedContentRef _Nullable FMGeneratedContentCreateFromJSON(const char *_Nonnull jsonString, int *_Nullable outErrorCode, char *_Nullable *_Nullable outErrorDescription);
char *_Nullable FMGeneratedContentGetJSONString(FMGeneratedContentRef _Nonnull content);
char *_Nullable FMGeneratedContentGetPropertyValue(FMGeneratedContentRef _Nonnull content, const char *_Nonnull propertyName, int *_Nullable outErrorCode, char *_Nullable *_Nullable outErrorDescription);
bool FMGeneratedContentIsComplete(FMGeneratedContentRef _Nonnull content);

// Structured generation session functions
FMTaskRef FMLanguageModelSessionRespondWithSchema(FMLanguageModelSessionRef _Nonnull session, FMComposedPrompt _Nonnull composedPrompt, FMGenerationSchemaRef _Nonnull schema, const char *_Nullable optionsJSON, void *_Nullable userInfo, FMLanguageModelSessionStructuredResponseCallback callback);
FMTaskRef FMLanguageModelSessionRespondWithSchemaFromJSON(FMLanguageModelSessionRef _Nonnull session, FMComposedPrompt _Nonnull composedPrompt, const char *_Nonnull schemaJSONString, const char *_Nullable optionsJSON, void *_Nullable userInfo, FMLanguageModelSessionStructuredResponseCallback callback);

// MARK: - Tools

// Tool functions
FMBridgedToolRef _Nullable FMBridgedToolCreate(const char *_Nonnull name, const char *_Nonnull description, FMGenerationSchemaRef _Nonnull parameters, void (*_Nonnull callable)(FMGeneratedContentRef _Nonnull, unsigned int), int *_Nullable outErrorCode, char *_Nullable *_Nullable outErrorDescription) __attribute__((swift_attr("@Sendable")));
void FMBridgedToolFinishCall(FMBridgedToolRef _Nonnull tool, unsigned int callId, const char *_Nonnull output);

// MARK: - Memory management

void FMTaskCancel(FMTaskRef task);

void FMRetain(const void *_Nonnull object);
void FMRelease(const void *_Nonnull object);
void FMFreeString(char *_Nullable str);

#endif /* FoundationModels_h */
