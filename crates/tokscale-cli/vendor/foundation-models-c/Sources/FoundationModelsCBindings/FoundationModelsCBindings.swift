/*
For licensing see accompanying LICENSE file.
Copyright (C) 2026 Apple Inc. All Rights Reserved.
*/

import Foundation
import FoundationModels
import FoundationModelsCDeclarations
import Synchronization

enum ComposedPromptError: Error {
  case unsupported
}

/// Builder class for a `Prompt`.
public class ComposedPrompt: NSObject, PromptRepresentable {
  public init(components: [PromptRepresentable] = []) {
    self.components = components
    super.init()
  }

  private(set) public var components: [PromptRepresentable]

  public func add(text: String) {
    self.components.append(text)
  }

  public func add(attachmentFromPath imagePath: String, label: String?) throws {
    // `Attachment` only exists in the macOS 27+ SDK
    #if FM_HAS_MACOS_27_SDK
    if #available(iOS 27.0, macOS 27.0, visionOS 27.0, watchOS 27.0, *) {
      let url = URL(fileURLWithPath: imagePath)
      var attachment = Attachment(imageURL: url)
      if let label {
        attachment = attachment.label(label)
      }
      self.components.append(attachment)
      return
    }
    #endif

    throw ComposedPromptError.unsupported
  }

  public var promptRepresentation: Prompt {
    return Prompt {
      components.map(\.promptRepresentation)
    }
  }
}

@_cdecl("FMComposedPromptInitialize")
public func FMComposedPromptInitialize() -> FMComposedPrompt {
  return FMComposedPrompt(Unmanaged.passRetained(ComposedPrompt()).toOpaque())
}

@_cdecl("FMComposedPromptAddText")
public func FMComposedPromptAddText(composedPrompt: FMComposedPrompt, text: UnsafePointer<CChar>) {
  let composedPrompt = Unmanaged<ComposedPrompt>.fromOpaque(composedPrompt).takeUnretainedValue()
  let textToAddToPrompt = String(cString: text)
  composedPrompt.add(text: textToAddToPrompt)
}

@_cdecl("FMComposedPromptAddAttachment")
public func FMComposedPromptAddAttachment(
  composedPrompt: FMComposedPrompt,
  imagePath: UnsafePointer<CChar>,
  label: UnsafePointer<CChar>?,
  error: UnsafeMutablePointer<FMComposedPromptAddImageError>?
) -> Bool {
  let composedPrompt = Unmanaged<ComposedPrompt>.fromOpaque(composedPrompt).takeUnretainedValue()
  let imageURLToAddToPrompt = String(cString: imagePath)
  let labelString = label.map(String.init(cString:))
  do {
    try composedPrompt.add(attachmentFromPath: imageURLToAddToPrompt, label: labelString)
    return true
  } catch ComposedPromptError.unsupported {
    error?.pointee = FMComposedPromptAddImageErrorUnsupported
    return false
  } catch _ {
    error?.pointee = FMComposedPromptAddImageErrorUnknown
    return false
  }
}

final class TaskBox {
  let task: Task<(), Never>
  init(_ task: Task<(), Never>) {
    self.task = task
  }
}

@_cdecl("FMSystemLanguageModelGetDefault")
public func FMSystemLanguageModelGetDefault() -> FMSystemLanguageModelRef {
  let model = SystemLanguageModel.default
  return FMSystemLanguageModelRef(Unmanaged.passRetained(model).toOpaque())
}

private extension SystemLanguageModel.UseCase {
  init(c: FMSystemLanguageModelUseCase) {
    let useCase: SystemLanguageModel.UseCase? =
      switch c {
      case FMSystemLanguageModelUseCaseGeneral: .general
      case FMSystemLanguageModelUseCaseContentTagging: .contentTagging
      default:
        nil
      }

    if useCase == nil {
      self = .general
      print("Warning: Unknown SystemLanguageModel use case \(c), defaulting to .general")
    } else {
      self = useCase!
    }
  }
}

extension SystemLanguageModel.Guardrails {
  init(c: FMSystemLanguageModelGuardrails) {
    self =
      switch c {
      case FMSystemLanguageModelGuardrailsDefault: .default
      case FMSystemLanguageModelGuardrailsPermissiveContentTransformations:
        .permissiveContentTransformations
      default:
        SystemLanguageModel.Guardrails.default
      }
  }
}

@_cdecl("FMSystemLanguageModelCreate")
public func FMSystemLanguageModelCreate(
  useCase: FMSystemLanguageModelUseCase,
  guardrails: FMSystemLanguageModelGuardrails
) -> FMSystemLanguageModelRef {
  let model = SystemLanguageModel(useCase: .init(c: useCase), guardrails: .init(c: guardrails))
  return FMSystemLanguageModelRef(Unmanaged.passRetained(model).toOpaque())
}

@_cdecl("FMSystemLanguageModelIsAvailable")
public func FMSystemLanguageModelIsAvailable(
  model: OpaquePointer,
  unavailableReason: UnsafeMutablePointer<FMSystemLanguageModelUnavailableReason>?
) -> Bool {
  let model = Unmanaged<SystemLanguageModel>.fromOpaque(UnsafeRawPointer(model))
    .takeUnretainedValue()
  switch model.availability {
  case .available:
    return true
  case .unavailable(.appleIntelligenceNotEnabled):
    unavailableReason?.pointee = FMSystemLanguageModelUnavailableReasonAppleIntelligenceNotEnabled
    return false
  case .unavailable(.deviceNotEligible):
    unavailableReason?.pointee = FMSystemLanguageModelUnavailableReasonDeviceNotEligible
    return false
  case .unavailable(.modelNotReady):
    unavailableReason?.pointee = FMSystemLanguageModelUnavailableReasonModelNotReady
    return false
  case .unavailable(_):
    unavailableReason?.pointee = FMSystemLanguageModelUnavailableReasonUnknown
    return false
  }
}

// MARK: - Session creation from SystemLanguageModel

@_cdecl("FMLanguageModelSessionCreateDefault")
public func FMLanguageModelSessionCreateDefault() -> FMLanguageModelSessionRef {
  let session = LanguageModelSession()
  return FMLanguageModelSessionRef(Unmanaged.passRetained(session).toOpaque())
}

@_cdecl("FMLanguageModelSessionCreateFromSystemLanguageModel")
public func FMLanguageModelSessionCreateFromSystemLanguageModel(
  model: UnsafePointer<FMSystemLanguageModelRef>?,
  instructions: UnsafePointer<CChar>?,
  tools: UnsafeMutablePointer<FMBridgedToolRef>?,
  toolCount: Int32
) -> FMLanguageModelSessionRef {
  var modelChoice: SystemLanguageModel
  if let model = model {
    modelChoice = Unmanaged<SystemLanguageModel>.fromOpaque(model).takeUnretainedValue()
  } else {
    modelChoice = SystemLanguageModel.default
  }

  // Convert the C array of tool refs to Swift array of Tool objects
  var toolArray: [any Tool] = []
  if let tools = tools, toolCount > 0 {
    for i in 0..<Int(toolCount) {
      let toolRef = tools[i]
      let bridgedTool = Unmanaged<BridgedTool>.fromOpaque(toolRef).takeUnretainedValue()
      toolArray.append(bridgedTool)
    }
  }

  let session = LanguageModelSession(
    model: modelChoice,
    tools: toolArray,
    instructions: instructions.map(String.init(cString:))
  )
  return FMLanguageModelSessionRef(Unmanaged.passRetained(session).toOpaque())
}

// MARK: - Session management

@_cdecl("FMLanguageModelSessionCreateFromTranscript")
public func FMLanguageModelSessionCreateFromTranscript(
  transcriptSession: FMLanguageModelSessionRef,
  model: UnsafePointer<FMSystemLanguageModelRef>?,
  tools: UnsafeMutablePointer<FMBridgedToolRef>?,
  toolCount: Int32
) -> FMLanguageModelSessionRef {
  // Extract the transcript from the existing session
  let existingSession = Unmanaged<LanguageModelSession>.fromOpaque(transcriptSession)
    .takeUnretainedValue()
  let transcript = existingSession.transcript

  // Get the model to use
  var modelChoice: SystemLanguageModel
  if let model = model {
    modelChoice = Unmanaged<SystemLanguageModel>.fromOpaque(model).takeUnretainedValue()
  } else {
    modelChoice = SystemLanguageModel.default
  }

  // Convert the C array of tool refs to Swift array of Tool objects
  var toolArray: [any Tool] = []
  if let tools = tools, toolCount > 0 {
    for i in 0..<Int(toolCount) {
      let toolRef = tools[i]
      let bridgedTool = Unmanaged<BridgedTool>.fromOpaque(toolRef).takeUnretainedValue()
      toolArray.append(bridgedTool)
    }
  }

  // Create a new session from the transcript
  let session = LanguageModelSession(
    model: modelChoice,
    tools: toolArray,
    transcript: transcript,
  )
  return FMLanguageModelSessionRef(Unmanaged.passRetained(session).toOpaque())
}

@_cdecl("FMLanguageModelSessionIsResponding")
public func FMLanguageModelSessionIsResponding(session: FMLanguageModelSessionRef) -> Bool {
  let session = Unmanaged<LanguageModelSession>.fromOpaque(session).takeUnretainedValue()
  return session.isResponding
}

/// Resets the task memory management state after a cancelled or failed request.
/// This ensures the session is ready to accept new requests.
///
/// This does NOT create a new session or clear the conversation transcript - it only
/// resets the internal task handling state to prepare for new requests.
///
/// - Parameter session: The language model session to reset task state for
///
/// - Note: This function is automatically called by the Python layer after request
///         cancellations or errors. It provides a hook for future native-level
///         task state cleanup enhancements.
@_cdecl("FMLanguageModelSessionReset")
public func FMLanguageModelSessionReset(session: FMLanguageModelSessionRef) {
  // The LanguageModelSession in Swift doesn't have an explicit task state reset method,
  // but we can ensure any pending operations are completed by checking isResponding.
  // This is a placeholder that can be enhanced if the Swift API adds task cleanup functionality.
  let session = Unmanaged<LanguageModelSession>.fromOpaque(session).takeUnretainedValue()

  // For now, this is a no-op as the Swift LanguageModelSession
  // should handle task cleanup internally. This function exists to provide
  // a hook for future improvements and to signal intent in the Python layer.
  _ = session.isResponding
}

private struct UnsafeSendableUserInfo: @unchecked Sendable {
  var pointer: UnsafeMutableRawPointer?
}

/// Error codes that match Python GenerationErrorCode enum
/// These codes are used across the C API to maintain consistency with Python bindings
private enum StatusCode: Int32 {
  case success = 0
  case exceededContextWindowSize = 1
  case assetsUnavailable = 2
  case guardrailViolation = 3
  case unsupportedGuide = 4
  case unsupportedLanguageOrLocale = 5
  case decodingFailure = 6
  case rateLimited = 7
  case concurrentRequests = 8
  case refusal = 9
  case invalidSchema = 10
  case invalidArgument = 11  // For NULL pointer errors (not in Python but useful for C API)
  case unknownError = 255
}

// Map LanguageModelSession.GenerationError to status codes that match Python API
private func mapGenerationErrorToStatusCode(_ error: LanguageModelSession.GenerationError) -> Int32
{
  switch error {
  case .exceededContextWindowSize:
    return StatusCode.exceededContextWindowSize.rawValue
  case .assetsUnavailable:
    return StatusCode.assetsUnavailable.rawValue
  case .guardrailViolation:
    return StatusCode.guardrailViolation.rawValue
  case .unsupportedGuide:
    return StatusCode.unsupportedGuide.rawValue
  case .unsupportedLanguageOrLocale:
    return StatusCode.unsupportedLanguageOrLocale.rawValue
  case .decodingFailure:
    return StatusCode.decodingFailure.rawValue
  case .rateLimited:
    return StatusCode.rateLimited.rawValue
  case .concurrentRequests:
    return StatusCode.concurrentRequests.rawValue
  case .refusal:
    return StatusCode.refusal.rawValue
  @unknown default:
    // Log unknown error types for debugging
    print("Warning: Unknown GenerationError type encountered: \(error)")
    print("  Error description: \(error.localizedDescription)")
    return StatusCode.unknownError.rawValue
  }
}

// Helper function to create detailed error descriptions from generic errors
private func formatErrorDescription(_ error: Error, function: String = #function) -> String {
  let nsError = error as NSError
  var debugDescription = "Error \(nsError.domain):\(nsError.code) - \(nsError.localizedDescription)"
  if !nsError.userInfo.isEmpty {
    debugDescription += " UserInfo: \(nsError.userInfo)"
  }

  #if DEBUG
  print("Unexpected error in \(function): \(debugDescription)")
  #endif

  return debugDescription
}

// MARK: - Session response

// Helper function to parse GenerationOptions from JSON
private func parseGenerationOptions(from jsonString: String?) throws -> GenerationOptions? {
  guard let jsonString = jsonString, !jsonString.isEmpty else {
    return nil
  }

  let data = Data(jsonString.utf8)
  guard let json = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
    throw NSError(
      domain: "GenerationOptions",
      code: -1,
      userInfo: [NSLocalizedDescriptionKey: "Invalid JSON"]
    )
  }

  var options = GenerationOptions()

  // Parse sampling mode
  if let samplingDict = json["sampling"] as? [String: Any],
    let mode = samplingDict["mode"] as? String
  {
    switch mode {
    case "greedy":
      options.sampling = .greedy
    case "random":
      let seed = samplingDict["seed"] as? UInt64
      // Swift API supports either topK or probabilityThreshold, not both
      if let topK = samplingDict["top_k"] as? Int {
        options.sampling = .random(top: topK, seed: seed)
      } else if let probabilityThreshold = samplingDict["top_p"] as? Double {
        options.sampling = .random(probabilityThreshold: probabilityThreshold, seed: seed)
      }
    default:
      break
    }
  }

  // Parse temperature
  if let temperature = json["temperature"] as? Double {
    options.temperature = temperature
  }

  // Parse maximum_response_tokens
  if let maxTokens = json["maximum_response_tokens"] as? Int {
    options.maximumResponseTokens = maxTokens
  }

  return options
}

@_cdecl("FMLanguageModelSessionRespond")
public func FMLanguageModelSessionRespond(
  session: FMLanguageModelSessionRef,
  composedPrompt: FMComposedPrompt,
  optionsJSON: UnsafePointer<CChar>?,
  userInfo: UnsafeMutableRawPointer?,
  callback: FMLanguageModelSessionResponseCallback
) -> FMTaskRef {
  let session = Unmanaged<LanguageModelSession>.fromOpaque(session).takeUnretainedValue()
  let unsafeSendableUserInfo = UnsafeSendableUserInfo(pointer: userInfo)
  let prompt = Unmanaged<ComposedPrompt>.fromOpaque(composedPrompt).takeUnretainedValue()
    .promptRepresentation
  let optionsJSONString = optionsJSON.map(String.init(cString:))

  let task = Task.detached {
    do {
      // Check cancellation at start
      try Task.checkCancellation()

      // Parse options if provided
      let options = try parseGenerationOptions(from: optionsJSONString)

      // Perform the expensive operation with options
      let response = try await session.respond(
        to: prompt,
        options: options ?? GenerationOptions()
      )

      // Check cancellation before callback
      try Task.checkCancellation()

      callback( /*status*/
        StatusCode.success.rawValue,
        response.content, /*length*/
        response.content.utf8.count,
        unsafeSendableUserInfo.pointer
      )
    } catch is CancellationError {
      // Handle cancellation explicitly
      let message = "Operation cancelled"
      callback(
        StatusCode.unknownError.rawValue,
        message,
        message.utf8.count,
        unsafeSendableUserInfo.pointer
      )
    } catch let error as LanguageModelSession.GenerationError {
      // Map specific generation errors to status codes
      let debugDescription = error.localizedDescription
      let statusCode = mapGenerationErrorToStatusCode(error)
      callback(
        statusCode,
        debugDescription,
        debugDescription.utf8.count,
        unsafeSendableUserInfo.pointer
      )
    } catch {
      // Generic error - unknown type
      let debugDescription = formatErrorDescription(error)
      callback(
        StatusCode.unknownError.rawValue,
        debugDescription,
        debugDescription.utf8.count,
        unsafeSendableUserInfo.pointer
      )
    }
  }
  let taskBox = TaskBox(task)
  return FMTaskRef(Unmanaged.passRetained(taskBox).toOpaque())
}

private final class UnsafeSendableResponseStreamBox<Content: Generable>: @unchecked Sendable {
  let stream: LanguageModelSession.ResponseStream<Content>
  let session: LanguageModelSession
  var iterationTask: Task<Void, Never>?

  init(stream: LanguageModelSession.ResponseStream<Content>, session: LanguageModelSession) {
    self.stream = stream
    self.session = session
  }

  deinit {
    // Cancel the iteration task when the stream box is deallocated
    // The task will check for cancellation and exit cleanly
    // Note: The task holds a strong reference to the session, so the session
    // won't be deallocated until the task completes
    iterationTask?.cancel()
  }
}

@_cdecl("FMLanguageModelSessionStreamResponse")
public func FMLanguageModelSessionStreamResponse(
  session: FMLanguageModelSessionRef,
  composedPrompt: FMComposedPrompt,
  optionsJSON: UnsafePointer<CChar>?
) -> FMLanguageModelSessionResponseStreamRef? {
  let session = Unmanaged<LanguageModelSession>.fromOpaque(session).takeUnretainedValue()
  let prompt = Unmanaged<ComposedPrompt>.fromOpaque(composedPrompt).takeUnretainedValue()
    .promptRepresentation
  let optionsJSONString = optionsJSON.map(String.init(cString:))

  do {
    let options = try parseGenerationOptions(from: optionsJSONString)
    let stream = session.streamResponse(to: prompt, options: options ?? GenerationOptions())
    let box = UnsafeSendableResponseStreamBox<String>(stream: stream, session: session)
    return FMLanguageModelSessionResponseStreamRef(Unmanaged.passRetained(box).toOpaque())
  } catch {
    // If parsing fails, return nil
    return nil
  }
}

@_cdecl("FMLanguageModelSessionResponseStreamIterate")
public func FMLanguageModelSessionResponseStreamIterate(
  stream: FMLanguageModelSessionResponseStreamRef,
  userInfo: UnsafeMutableRawPointer?,
  callback: FMLanguageModelSessionResponseCallback
) {
  let streamBox = Unmanaged<UnsafeSendableResponseStreamBox<String>>.fromOpaque(stream)
    .takeUnretainedValue()
  let unsafeSendableUserInfo = UnsafeSendableUserInfo(pointer: userInfo)

  // Capture both the session and stream in the task closure to create strong references
  // This prevents them from being deallocated while the task is running
  let task = Task.detached { [session = streamBox.session, stream = streamBox.stream] in
    do {
      // Check cancellation at start
      try Task.checkCancellation()

      for try await snapshot in stream {
        // Check cancellation before each callback
        try Task.checkCancellation()
        snapshot.content.withCString { [length = snapshot.content.utf8.count] cString in
          callback( /*status*/
            StatusCode.success.rawValue, /*content*/
            cString,
            length,
            unsafeSendableUserInfo.pointer
          )
        }
      }

      // Final callback to signal completion
      callback( /*status*/
        StatusCode.success.rawValue, /*content*/
        nil, /*length*/
        0,
        unsafeSendableUserInfo.pointer
      )
    } catch is CancellationError {
      // Handle cancellation explicitly
      let message = "Stream cancelled"
      callback(
        StatusCode.unknownError.rawValue,
        message,
        message.utf8.count,
        unsafeSendableUserInfo.pointer
      )
    } catch let error as LanguageModelSession.GenerationError {
      // Map specific generation errors to status codes
      let statusCode = mapGenerationErrorToStatusCode(error)
      let debugDescription = error.localizedDescription
      callback(
        statusCode,
        debugDescription,
        debugDescription.utf8.count,
        unsafeSendableUserInfo.pointer
      )
    } catch {
      // Generic error - unknown type
      let debugDescription = formatErrorDescription(error)
      callback(
        StatusCode.unknownError.rawValue,
        debugDescription,
        debugDescription.utf8.count,
        unsafeSendableUserInfo.pointer
      )
    }

    // Keep the session and stream references alive until the task completes
    _ = session
    _ = stream
  }

  // Store the task in the stream box so it can be cancelled on dealloc
  streamBox.iterationTask = task
}

@_cdecl("FMLanguageModelSessionRespondWithSchema")
public func FMLanguageModelSessionRespondWithSchema(
  session: FMLanguageModelSessionRef,
  composedPrompt: FMComposedPrompt,
  schema: FMGenerationSchemaRef,
  optionsJSON: UnsafePointer<CChar>?,
  userInfo: UnsafeMutableRawPointer?,
  callback: FMLanguageModelSessionStructuredResponseCallback
) -> FMTaskRef {
  let session = Unmanaged<LanguageModelSession>.fromOpaque(session).takeUnretainedValue()
  let prompt = Unmanaged<ComposedPrompt>.fromOpaque(composedPrompt).takeUnretainedValue()
    .promptRepresentation
  let schemaBuilder = Unmanaged<GenerationSchemaBuilder>.fromOpaque(schema).takeUnretainedValue()
  let optionsJSONString = optionsJSON.map(String.init(cString:))
  let unsafeSendableUserInfo = UnsafeSendableUserInfo(pointer: userInfo)

  let task = Task.detached {
    do {
      // Check cancellation at start
      try Task.checkCancellation()

      // Build the final schema from the builder
      let finalSchema = try schemaBuilder.buildSchema()

      // Parse options if provided
      let options = try parseGenerationOptions(from: optionsJSONString)

      // Use Foundation Models guided generation API
      try Task.checkCancellation()
      let response = try await session.respond(
        to: prompt,
        schema: finalSchema,
        options: options ?? GenerationOptions()
      )

      // Check cancellation before callback
      try Task.checkCancellation()

      let contentWrapper = GeneratedContentWrapper(content: response.content)
      let contentRef = FMGeneratedContentRef(Unmanaged.passRetained(contentWrapper).toOpaque())
      callback( /*status*/StatusCode.success.rawValue, contentRef, unsafeSendableUserInfo.pointer)
    } catch is CancellationError {
      // Handle cancellation explicitly
      let message = "Operation cancelled"
      let contentWrapper = GeneratedContentWrapper(content: message)
      let contentRef = FMGeneratedContentRef(Unmanaged.passRetained(contentWrapper).toOpaque())
      callback(
        StatusCode.unknownError.rawValue,
        contentRef,
        unsafeSendableUserInfo.pointer
      )
    } catch let error as LanguageModelSession.GenerationError {
      // Map specific generation errors to status codes
      let statusCode = mapGenerationErrorToStatusCode(error)
      let contentWrapper = GeneratedContentWrapper(content: error.localizedDescription)
      let contentRef = FMGeneratedContentRef(Unmanaged.passRetained(contentWrapper).toOpaque())
      callback(statusCode, contentRef, unsafeSendableUserInfo.pointer)
    } catch {
      // Generic error - unknown type
      let debugDescription = formatErrorDescription(error)
      let contentWrapper = GeneratedContentWrapper(content: debugDescription)
      let contentRef = FMGeneratedContentRef(Unmanaged.passRetained(contentWrapper).toOpaque())
      callback(
        StatusCode.unknownError.rawValue,
        contentRef,
        unsafeSendableUserInfo.pointer
      )
    }
  }
  let taskBox = TaskBox(task)
  return FMTaskRef(Unmanaged.passRetained(taskBox).toOpaque())
}

@_cdecl("FMLanguageModelSessionRespondWithSchemaFromJSON")
public func FMLanguageModelSessionRespondWithSchemaFromJSON(
  session: FMLanguageModelSessionRef,
  composedPrompt: FMComposedPrompt,
  jsonSchema: UnsafePointer<CChar>,
  optionsJSON: UnsafePointer<CChar>?,
  userInfo: UnsafeMutableRawPointer?,
  callback: FMLanguageModelSessionStructuredResponseCallback
) -> FMTaskRef {
  let session = Unmanaged<LanguageModelSession>.fromOpaque(session).takeUnretainedValue()
  let prompt = Unmanaged<ComposedPrompt>.fromOpaque(composedPrompt).takeUnretainedValue()
    .promptRepresentation
  let jsonSchemaString = String(cString: jsonSchema)
  let optionsJSONString = optionsJSON.map(String.init(cString:))
  let unsafeSendableUserInfo = UnsafeSendableUserInfo(pointer: userInfo)

  let task = Task.detached {
    do {
      // Check cancellation at start
      try Task.checkCancellation()

      // Use Foundation Models guided generation API with JSON schema
      let schema = try JSONDecoder().decode(
        GenerationSchema.self,
        from: Data(jsonSchemaString.utf8)
      )

      // Parse options if provided
      let options = try parseGenerationOptions(from: optionsJSONString)

      try Task.checkCancellation()
      let response = try await session.respond(
        to: prompt,
        schema: schema,
        options: options ?? GenerationOptions()
      )

      // Check cancellation before callback
      try Task.checkCancellation()

      let contentWrapper = GeneratedContentWrapper(content: response.content)
      let contentRef = FMGeneratedContentRef(Unmanaged.passRetained(contentWrapper).toOpaque())
      callback( /*status*/StatusCode.success.rawValue, contentRef, unsafeSendableUserInfo.pointer)
    } catch is CancellationError {
      // Handle cancellation explicitly
      let message = "Operation cancelled"
      let contentWrapper = GeneratedContentWrapper(content: message)
      let contentRef = FMGeneratedContentRef(Unmanaged.passRetained(contentWrapper).toOpaque())
      callback(
        StatusCode.unknownError.rawValue,
        contentRef,
        unsafeSendableUserInfo.pointer
      )
    } catch let error as LanguageModelSession.GenerationError {
      // Map specific generation errors to status codes
      let statusCode = mapGenerationErrorToStatusCode(error)
      let contentWrapper = GeneratedContentWrapper(content: error.localizedDescription)
      let contentRef = FMGeneratedContentRef(Unmanaged.passRetained(contentWrapper).toOpaque())
      callback(statusCode, contentRef, unsafeSendableUserInfo.pointer)
    } catch {
      // Generic error - unknown type
      let debugDescription = formatErrorDescription(error)
      let contentWrapper = GeneratedContentWrapper(content: debugDescription)
      let contentRef = FMGeneratedContentRef(Unmanaged.passRetained(contentWrapper).toOpaque())
      callback(
        StatusCode.unknownError.rawValue,
        contentRef,
        unsafeSendableUserInfo.pointer
      )
    }
  }
  let taskBox = TaskBox(task)
  return FMTaskRef(Unmanaged.passRetained(taskBox).toOpaque())
}

// MARK: - Transcript

@_cdecl("FMTranscriptCreateFromJSONString")
public func FMTranscriptCreateFromJSONString(
  jsonString: UnsafePointer<CChar>,
  outErrorCode: UnsafeMutablePointer<Int32>?,
  outErrorDescription: UnsafeMutablePointer<UnsafePointer<CChar>?>?
) -> FMLanguageModelSessionRef? {
  let jsonStr = String(cString: jsonString)

  do {
    let transcript = try JSONDecoder().decode(Transcript.self, from: Data(jsonStr.utf8))
    // Create a new session initialized with the transcript
    let session = LanguageModelSession(transcript: transcript)
    return FMLanguageModelSessionRef(Unmanaged.passRetained(session).toOpaque())
  } catch {
    let debugDescription = formatErrorDescription(error)
    debugDescription.withCString { cString in
      outErrorCode?.pointee = StatusCode.decodingFailure.rawValue
      outErrorDescription?.pointee = UnsafePointer(strdup(cString))
    }
    return nil
  }
}

/// Returns a JSON string representation of the session transcript.
///
/// - Parameters:
///   - session: The language model session
///   - outErrorCode: Optional pointer to receive error code on failure
///   - outErrorDescription: Optional pointer to receive error description on failure
///
/// - Returns: A C string containing JSON, or NULL on error
///
/// - Important: The returned string is allocated with malloc and MUST be freed
///              by calling FMFreeString() when no longer needed to prevent memory leaks.
///
/// - Note: On error, if outErrorDescription is provided, it will also contain an allocated
///         string that must be freed with FMFreeString().
@_cdecl("FMLanguageModelSessionGetTranscriptJSONString")
public func FMLanguageModelSessionGetTranscriptJSONString(
  session: FMLanguageModelSessionRef,
  outErrorCode: UnsafeMutablePointer<Int32>?,
  outErrorDescription: UnsafeMutablePointer<UnsafePointer<CChar>?>?
) -> UnsafeMutablePointer<CChar>? {
  let session = Unmanaged<LanguageModelSession>.fromOpaque(session).takeUnretainedValue()

  do {
    let transcript_raw = session.transcript
    let json = try JSONEncoder().encode(transcript_raw)
    let transcript = String(data: json, encoding: .utf8)!
    return transcript.withCString { cString in
      return UnsafeMutablePointer(strdup(cString))
    }
  } catch let error as LanguageModelSession.GenerationError {
    // Map specific generation errors to error codes
    let errorCode = mapGenerationErrorToStatusCode(error)
    let debugDescription = error.localizedDescription
    debugDescription.withCString { cString in
      outErrorCode?.pointee = errorCode
      outErrorDescription?.pointee = UnsafePointer(strdup(cString))
    }
    return nil
  } catch {
    // Generic error - unknown type
    let debugDescription = error.localizedDescription
    debugDescription.withCString { cString in
      outErrorCode?.pointee = StatusCode.unknownError.rawValue
      outErrorDescription?.pointee = UnsafePointer(strdup(cString))
    }
    return nil
  }
}

// MARK: - Task management

@_cdecl("FMTaskCancel")
public func FMTaskCancel(_ task: FMTaskRef) {
  Unmanaged<TaskBox>.fromOpaque(task).takeUnretainedValue().task.cancel()
}

@_cdecl("FMRetain")
public func FMRetain(_ object: UnsafeRawPointer) {
  _ = Unmanaged<AnyObject>.fromOpaque(object).retain()
}

@_cdecl("FMRelease")
public func FMRelease(_ object: UnsafeRawPointer) {
  Unmanaged<AnyObject>.fromOpaque(object).release()
}

/// Frees a string allocated by Foundation Models C API.
///
/// Many C API functions return strings allocated with malloc (via strdup).
/// These strings must be freed by calling this function when no longer needed
/// to prevent memory leaks.
///
/// - Parameter ptr: The string pointer to free (may be NULL, in which case this is a no-op)
///
/// - Note: Only call this on strings returned by Foundation Models C API functions
///         that explicitly document the need to free the returned string.
@_cdecl("FMFreeString")
public func FMFreeString(_ ptr: UnsafeMutablePointer<CChar>?) {
  free(ptr)
}

// MARK: - Schema creation

@_cdecl("FMGenerationSchemaCreate")
public func FMGenerationSchemaCreate(
  name: UnsafePointer<CChar>,
  description: UnsafePointer<CChar>?
)
  -> FMGenerationSchemaRef
{
  let descriptionString = description.map(String.init(cString:))
  let nameString = String(cString: name)

  // Create a mutable schema builder that can accumulate properties
  let builder = GenerationSchemaBuilder(name: nameString, description: descriptionString)
  return FMGenerationSchemaRef(Unmanaged.passRetained(builder).toOpaque())
}

@_cdecl("FMGenerationSchemaPropertyCreate")
public func FMGenerationSchemaPropertyCreate(
  name: UnsafePointer<CChar>,
  description: UnsafePointer<CChar>?,
  typeName: UnsafePointer<CChar>,
  isOptional: Bool = false
) -> FMGenerationSchemaPropertyRef {
  let nameString = String(cString: name)
  let descriptionString = description.map(String.init(cString:))
  let typeNameString = String(cString: typeName)

  // Create a property info struct
  let propertyInfo = PropertyInfo(
    name: nameString,
    description: descriptionString,
    typeName: typeNameString,
    isOptional: isOptional
  )
  return FMGenerationSchemaPropertyRef(Unmanaged.passRetained(propertyInfo).toOpaque())
}

// MARK: - Guides

@_cdecl("FMGenerationSchemaPropertyAddAnyOfGuide")
public func FMGenerationSchemaPropertyAddAnyOfGuide(
  property: FMGenerationSchemaPropertyRef,
  anyOf: UnsafeMutablePointer<UnsafePointer<CChar>?>,
  choiceCount: Int32,
  wrapped: Bool = false
) {
  let propertyInfo = Unmanaged<PropertyInfo>.fromOpaque(property).takeUnretainedValue()

  // Convert C string array to Swift string array
  var choiceStrings: [String] = []
  for i in 0..<Int(choiceCount) {
    if let choicePtr = anyOf[i] {
      choiceStrings.append(String(cString: choicePtr))
    }
  }

  // Add anyOf guide to the property
  if wrapped {
    let elementGuide = PropertyGuide.anyOf(choiceStrings)
    propertyInfo.guides.append(.element(elementGuide))
  } else {
    propertyInfo.guides.append(.anyOf(choiceStrings))
  }
}

@_cdecl("FMGenerationSchemaPropertyAddCountGuide")
public func FMGenerationSchemaPropertyAddCountGuide(
  property: FMGenerationSchemaPropertyRef,
  count: Int32,
  wrapped: Bool = false
) {
  let propertyInfo = Unmanaged<PropertyInfo>.fromOpaque(property).takeUnretainedValue()
  if wrapped {
    // For wrapped count guide, we need to wrap it in an element guide
    let elementGuide = PropertyGuide.count(Int(count))
    propertyInfo.guides.append(.element(elementGuide))
  } else {
    propertyInfo.guides.append(.count(Int(count)))
  }
}

@_cdecl("FMGenerationSchemaPropertyAddMaximumGuide")
public func FMGenerationSchemaPropertyAddMaximumGuide(
  property: FMGenerationSchemaPropertyRef,
  maximum: Double,
  wrapped: Bool = false
) {
  let propertyInfo = Unmanaged<PropertyInfo>.fromOpaque(property).takeUnretainedValue()
  if wrapped {
    let elementGuide = PropertyGuide.maximum(maximum)
    propertyInfo.guides.append(.element(elementGuide))
  } else {
    propertyInfo.guides.append(.maximum(maximum))
  }
}

@_cdecl("FMGenerationSchemaPropertyAddMinimumGuide")
public func FMGenerationSchemaPropertyAddMinimumGuide(
  property: FMGenerationSchemaPropertyRef,
  minimum: Double,
  wrapped: Bool = false
) {
  let propertyInfo = Unmanaged<PropertyInfo>.fromOpaque(property).takeUnretainedValue()
  if wrapped {
    let elementGuide = PropertyGuide.minimum(minimum)
    propertyInfo.guides.append(.element(elementGuide))
  } else {
    propertyInfo.guides.append(.minimum(minimum))
  }
}

@_cdecl("FMGenerationSchemaPropertyAddMinItemsGuide")
public func FMGenerationSchemaPropertyAddMinItemsGuide(
  property: FMGenerationSchemaPropertyRef,
  minItems: Int32
) {
  let propertyInfo = Unmanaged<PropertyInfo>.fromOpaque(property).takeUnretainedValue()
  propertyInfo.guides.append(.minItems(Int(minItems)))
}

@_cdecl("FMGenerationSchemaPropertyAddMaxItemsGuide")
public func FMGenerationSchemaPropertyAddMaxItemsGuide(
  property: FMGenerationSchemaPropertyRef,
  maxItems: Int32
) {
  let propertyInfo = Unmanaged<PropertyInfo>.fromOpaque(property).takeUnretainedValue()
  propertyInfo.guides.append(.maxItems(Int(maxItems)))
}

@_cdecl("FMGenerationSchemaPropertyAddRangeGuide")
public func FMGenerationSchemaPropertyAddRangeGuide(
  property: FMGenerationSchemaPropertyRef,
  minValue: Double,
  maxValue: Double,
  wrapped: Bool = false
) {
  let propertyInfo = Unmanaged<PropertyInfo>.fromOpaque(property).takeUnretainedValue()
  if wrapped {
    let elementGuide = PropertyGuide.range(min: minValue, max: maxValue)
    propertyInfo.guides.append(.element(elementGuide))
  } else {
    propertyInfo.guides.append(.range(min: minValue, max: maxValue))
  }
}

@_cdecl("FMGenerationSchemaPropertyAddRegex")
public func FMGenerationSchemaPropertyAddRegex(
  property: FMGenerationSchemaPropertyRef,
  pattern: UnsafePointer<CChar>,
  wrapped: Bool = false
) {
  let propertyInfo = Unmanaged<PropertyInfo>.fromOpaque(property).takeUnretainedValue()
  let patternString = String(cString: pattern)
  if wrapped {
    let elementGuide = PropertyGuide.regex(patternString)
    propertyInfo.guides.append(.element(elementGuide))
  } else {
    propertyInfo.guides.append(.regex(patternString))
  }
}

@_cdecl("FMGenerationSchemaAddProperty")
public func FMGenerationSchemaAddProperty(
  schema: FMGenerationSchemaRef,
  property: FMGenerationSchemaPropertyRef
) {
  let builder = Unmanaged<GenerationSchemaBuilder>.fromOpaque(schema).takeUnretainedValue()
  let propertyInfo = Unmanaged<PropertyInfo>.fromOpaque(property).takeUnretainedValue()

  // Add the property to the builder
  builder.addProperty(propertyInfo)
}

@_cdecl("FMGenerationSchemaAddReferenceSchema")
public func FMGenerationSchemaAddReferenceSchema(
  schema: FMGenerationSchemaRef,
  referenceSchema: FMGenerationSchemaRef
) {
  let builder = Unmanaged<GenerationSchemaBuilder>.fromOpaque(schema).takeUnretainedValue()
  let referenceBuilder = Unmanaged<GenerationSchemaBuilder>.fromOpaque(referenceSchema)
    .takeUnretainedValue()

  // Add the reference schema to the builder
  builder.addReferenceSchema(referenceBuilder)
}

// MARK: - Schema JSON

/// Returns a JSON string representation of the generation schema.
///
/// - Parameters:
///   - schema: The generation schema
///   - outErrorCode: Optional pointer to receive error code on failure
///   - outErrorDescription: Optional pointer to receive error description on failure
///
/// - Returns: A C string containing JSON, or NULL on error
///
/// - Important: The returned string is allocated with malloc and MUST be freed
///              by calling FMFreeString() when no longer needed to prevent memory leaks.
///
/// - Note: On error, if outErrorDescription is provided, it will also contain an allocated
///         string that must be freed with FMFreeString().
@_cdecl("FMGenerationSchemaGetJSONString")
public func FMGenerationSchemaGetJSONString(
  schema: FMGenerationSchemaRef,
  outErrorCode: UnsafeMutablePointer<Int32>?,
  outErrorDescription: UnsafeMutablePointer<UnsafePointer<CChar>?>?
) -> UnsafeMutablePointer<CChar>? {
  do {
    let builder = Unmanaged<GenerationSchemaBuilder>.fromOpaque(schema).takeUnretainedValue()
    let generationSchema = try builder.buildSchema()
    let json = generationSchema.debugDescription
    return json.withCString { cString in
      return UnsafeMutablePointer(strdup(cString))
    }
  } catch {
    let debugDescription = error.localizedDescription
    debugDescription.withCString { cString in
      outErrorCode?.pointee = StatusCode.invalidSchema.rawValue
      outErrorDescription?.pointee = UnsafePointer(strdup(cString))
    }
    return nil
  }
}

// MARK: - GeneratedContent functions

@_cdecl("FMGeneratedContentCreateFromJSON")
public func FMGeneratedContentCreateFromJSON(
  jsonString: UnsafePointer<CChar>,
  outErrorCode: UnsafeMutablePointer<Int32>?,
  outErrorDescription: UnsafeMutablePointer<UnsafePointer<CChar>?>?
) -> FMGeneratedContentRef? {
  let jsonStr = String(cString: jsonString)

  do {
    // Use Foundation Models GeneratedContent API
    let content = try GeneratedContent(json: jsonStr)
    let wrapper = GeneratedContentWrapper(content: content)
    return FMGeneratedContentRef(Unmanaged.passRetained(wrapper).toOpaque())
  } catch let error as LanguageModelSession.GenerationError {
    // Map specific generation errors to error codes
    let errorCode = mapGenerationErrorToStatusCode(error)
    let debugDescription = error.localizedDescription
    debugDescription.withCString { cString in
      outErrorCode?.pointee = errorCode
      outErrorDescription?.pointee = UnsafePointer(strdup(cString))
    }
    return nil
  } catch {
    // Generic error - unknown type
    let debugDescription = error.localizedDescription
    debugDescription.withCString { cString in
      outErrorCode?.pointee = StatusCode.unknownError.rawValue
      outErrorDescription?.pointee = UnsafePointer(strdup(cString))
    }
    return nil
  }
}

// MARK: - GeneratedContent

/// Returns a JSON string representation of the generated content.
///
/// - Parameter content: The generated content
///
/// - Returns: A C string containing JSON
///
/// - Important: The returned string is allocated with malloc and MUST be freed
///              by calling FMFreeString() when no longer needed to prevent memory leaks.
@_cdecl("FMGeneratedContentGetJSONString")
public func FMGeneratedContentGetJSONString(
  content: FMGeneratedContentRef
) -> UnsafeMutablePointer<CChar>? {
  let wrapper = Unmanaged<GeneratedContentWrapper>.fromOpaque(content).takeUnretainedValue()

  // Get JSON representation from Foundation Models GeneratedContent
  let jsonString = wrapper.content.jsonString
  return jsonString.withCString { cString in
    return UnsafeMutablePointer(strdup(cString))
  }
}

/// Returns the value of a specific property from the generated content.
///
/// - Parameters:
///   - content: The generated content
///   - propertyName: The name of the property to retrieve
///   - outErrorCode: Optional pointer to receive error code on failure
///   - outErrorDescription: Optional pointer to receive error description on failure
///
/// - Returns: A C string containing the property value, or NULL on error
///
/// - Important: The returned string is allocated with malloc and MUST be freed
///              by calling FMFreeString() when no longer needed to prevent memory leaks.
///
/// - Note: On error, if outErrorDescription is provided, it will also contain an allocated
///         string that must be freed with FMFreeString().
@_cdecl("FMGeneratedContentGetPropertyValue")
public func FMGeneratedContentGetPropertyValue(
  content: FMGeneratedContentRef,
  propertyName: UnsafePointer<CChar>,
  outErrorCode: UnsafeMutablePointer<Int32>?,
  outErrorDescription: UnsafeMutablePointer<UnsafePointer<CChar>?>?
) -> UnsafeMutablePointer<CChar>? {
  let wrapper = Unmanaged<GeneratedContentWrapper>.fromOpaque(content).takeUnretainedValue()
  let propName = String(cString: propertyName)
  // Use Foundation Models API to get property value
  do {
    let value: String = try wrapper.content.value(forProperty: propName)
    return value.withCString { cString in
      return UnsafeMutablePointer(strdup(cString))
    }
  } catch let error as LanguageModelSession.GenerationError {
    // Map specific generation errors to error codes
    let errorCode = mapGenerationErrorToStatusCode(error)
    let debugDescription = error.localizedDescription
    debugDescription.withCString { cString in
      outErrorCode?.pointee = errorCode
      outErrorDescription?.pointee = UnsafePointer(strdup(cString))
    }
    return nil
  } catch {
    // Generic error - unknown type
    let debugDescription = error.localizedDescription
    debugDescription.withCString { cString in
      outErrorCode?.pointee = StatusCode.unknownError.rawValue
      outErrorDescription?.pointee = UnsafePointer(strdup(cString))
    }
    return nil
  }
}

@_cdecl("FMGeneratedContentIsComplete")
public func FMGeneratedContentIsComplete(content: FMGeneratedContentRef) -> Bool {
  let wrapper = Unmanaged<GeneratedContentWrapper>.fromOpaque(content).takeUnretainedValue()
  return wrapper.content.isComplete
}

// MARK: - Wrapper classes for C bindings

private final class GenerationSchemaWrapper: @unchecked Sendable {
  let schema: GenerationSchema

  init(schema: GenerationSchema) {
    self.schema = schema
  }
}

private final class GenerationSchemaPropertyWrapper: @unchecked Sendable {
  let property: GenerationSchema.Property

  init(property: GenerationSchema.Property) {
    self.property = property
  }
}

private final class GeneratedContentWrapper: @unchecked Sendable {
  let content: GeneratedContent

  init(content: GeneratedContent) {
    self.content = content
  }

  init(content: String) {
    self.content = GeneratedContent(content)
  }
}

// MARK: - Schema building helper classes

private final class PropertyInfo: @unchecked Sendable {
  let name: String
  let description: String?
  let typeName: String
  let isOptional: Bool
  var guides: [PropertyGuide] = []

  init(name: String, description: String?, typeName: String, isOptional: Bool = false) {
    self.name = name
    self.description = description
    self.typeName = typeName
    self.isOptional = isOptional
  }
}

private indirect enum PropertyGuide: @unchecked Sendable {
  case anyOf([String])
  case count(Int)
  case element(Self)  // wraps guides for elements in an array
  case maxItems(Int)
  case maximum(Double)
  case minItems(Int)
  case minimum(Double)
  case range(min: Double, max: Double)
  case regex(String)
}

// MARK: - Guide builders

private func resolveStringGuides(
  _ guide: PropertyGuide
) throws -> GenerationGuide<String> {
  switch guide {
  case .anyOf(let anyOf):
    return GenerationGuide.anyOf(anyOf)
  case .regex(let pattern):
    // Create a Regex object from the pattern string
    let regex = try Regex(pattern)
    return GenerationGuide.pattern(regex)
  default:
    let context = LanguageModelSession.GenerationError.Context(
      debugDescription: "Unsupported guide for string type"
    )
    throw LanguageModelSession.GenerationError.unsupportedGuide(context)
  }
}

private func resolveArrayStringGuides(
  _ guide: PropertyGuide
) throws -> GenerationGuide<[String]> {
  switch guide {
  case .count(let count):
    return GenerationGuide.count(count)
  case .maxItems(let count):
    return GenerationGuide.maximumCount(count)
  case .minItems(let count):
    return GenerationGuide.minimumCount(count)
  case .anyOf(let anyOf):
    return GenerationGuide.element(GenerationGuide.anyOf(anyOf))
  case .element(let wrapped):
    let elementGuide = try resolveStringGuides(wrapped)
    return GenerationGuide.element(elementGuide)
  default:
    let context = LanguageModelSession.GenerationError.Context(
      debugDescription: "Unsupported guide type"
    )
    throw LanguageModelSession.GenerationError.unsupportedGuide(context)
  }
}

private func resolveDoubleGuides(
  _ guide: PropertyGuide
) throws -> GenerationGuide<Double> {
  switch guide {
  case .range(let min, let max):
    return GenerationGuide.range(min...max)
  case .maximum(let max):
    return GenerationGuide.maximum(max)
  case .minimum(let min):
    return GenerationGuide.minimum(min)
  default:
    let context = LanguageModelSession.GenerationError.Context(
      debugDescription: "Unsupported guide for double type"
    )
    throw LanguageModelSession.GenerationError.unsupportedGuide(context)
  }
}

private func resolveIntGuides(
  _ guide: PropertyGuide
) throws -> GenerationGuide<Int> {
  switch guide {
  case .range(let min, let max):
    return GenerationGuide.range(Int(min)...Int(max))
  case .maximum(let max):
    return GenerationGuide.maximum(Int(max))
  case .minimum(let min):
    return GenerationGuide.minimum(Int(min))
  default:
    let context = LanguageModelSession.GenerationError.Context(
      debugDescription: "Unsupported guide for int type"
    )
    throw LanguageModelSession.GenerationError.unsupportedGuide(context)
  }
}

private func resolveIntArrayGuides(
  _ guide: PropertyGuide
) throws -> GenerationGuide<[Int]> {
  switch guide {
  case .count(let count):
    return GenerationGuide.count(count)
  case .maxItems(let count):
    return GenerationGuide.maximumCount(count)
  case .minItems(let count):
    return GenerationGuide.minimumCount(count)
  case .element(let wrapped):
    let elementGuide = try resolveIntGuides(wrapped)
    return GenerationGuide.element(elementGuide)
  default:
    let context = LanguageModelSession.GenerationError.Context(
      debugDescription: "Unsupported guide for array<integer> type"
    )
    throw LanguageModelSession.GenerationError.unsupportedGuide(context)
  }
}

private func resolveDoubleArrayGuides(
  _ guide: PropertyGuide
) throws -> GenerationGuide<[Double]> {
  switch guide {
  case .count(let count):
    return GenerationGuide.count(count)
  case .maxItems(let count):
    return GenerationGuide.maximumCount(count)
  case .minItems(let count):
    return GenerationGuide.minimumCount(count)
  case .element(let wrapped):
    let elementGuide = try resolveDoubleGuides(wrapped)
    return GenerationGuide.element(elementGuide)
  default:
    let context = LanguageModelSession.GenerationError.Context(
      debugDescription: "Unsupported guide for array<number> type"
    )
    throw LanguageModelSession.GenerationError.unsupportedGuide(context)
  }
}

// MARK: - Schema builder

private final class GenerationSchemaBuilder: @unchecked Sendable {
  let description: String?
  let name: String  // Needed for self-nested definitions
  private var properties: [PropertyInfo] = []
  private var referenceSchemas: [GenerationSchemaBuilder] = []

  init(name: String, description: String?) {
    self.name = name
    self.description = description
  }

  func addProperty(_ property: PropertyInfo) {
    properties.append(property)
  }

  func addReferenceSchema(_ schema: GenerationSchemaBuilder) {
    referenceSchemas.append(schema)
  }

  func buildSchema() throws -> GenerationSchema {
    // Build reference schemas
    let refSchemas = try referenceSchemas.map {
      try $0.buildDynamicSchema()
    }

    // Build dynamic schema
    let dynamicSchema = try buildDynamicSchema()

    // Create the final schema - we need a Generable type
    // Let's use String as a base type and build properties dynamically
    let schema = try GenerationSchema(
      root: dynamicSchema,
      dependencies: refSchemas
    )

    return schema
  }

  func buildDynamicSchema() throws -> DynamicGenerationSchema {
    // Convert PropertyInfo to DynamicGenerationSchema.Property with correct syntax
    let schemaProperties = try properties.map {
      try buildDynamicSchemaProperty($0)
    }

    return DynamicGenerationSchema(
      name: name,
      description: description,
      properties: schemaProperties
    )
  }

  func buildDynamicSchemaProperty(
    _ propertyInfo: PropertyInfo
  ) throws
    -> DynamicGenerationSchema.Property
  {
    // Map type names to Swift types and create properties accordingly
    switch propertyInfo.typeName {
    case "string":
      // Convert PropertyGuide to String-specific GenerationGuide
      let stringGuides = try propertyInfo.guides.compactMap {
        try resolveStringGuides($0)
      }
      return DynamicGenerationSchema.Property(
        name: propertyInfo.name,
        description: propertyInfo.description,
        schema: .init(
          type: String.self,
          guides: stringGuides
        ),
        isOptional: propertyInfo.isOptional
      )
    case "number", "float", "double":
      // Convert PropertyGuide to Double-specific GenerationGuide
      let doubleGuides = try propertyInfo.guides.compactMap {
        try resolveDoubleGuides($0)
      }
      return DynamicGenerationSchema.Property(
        name: propertyInfo.name,
        description: propertyInfo.description,
        schema: .init(type: Double.self, guides: doubleGuides),
        isOptional: propertyInfo.isOptional
      )
    case "integer", "int":
      // Convert PropertyGuide to Int-specific GenerationGuide
      let intGuides = try propertyInfo.guides.compactMap { guide -> GenerationGuide<Int>? in
        try resolveIntGuides(guide)
      }
      return DynamicGenerationSchema.Property(
        name: propertyInfo.name,
        description: propertyInfo.description,
        schema: .init(type: Int.self, guides: intGuides),
        isOptional: propertyInfo.isOptional
      )
    case "boolean", "bool":
      return DynamicGenerationSchema.Property(
        name: propertyInfo.name,
        description: propertyInfo.description,
        schema: .init(type: Bool.self),  // bool has no guides
        isOptional: propertyInfo.isOptional
      )
    case let typeName where typeName.starts(with: "array<"):
      // Handle array types like "array<string>", "array<integer>", etc.
      if typeName == "array<string>" {
        // Convert PropertyGuide to [String]-specific GenerationGuide
        let arrayGuides = try propertyInfo.guides.compactMap {
          try resolveArrayStringGuides($0)
        }
        return DynamicGenerationSchema.Property(
          name: propertyInfo.name,
          description: propertyInfo.description,
          schema: .init(type: [String].self, guides: arrayGuides),
          isOptional: propertyInfo.isOptional
        )
      } else if typeName == "array<integer>" {
        // Handle array of integers
        let arrayGuides = try propertyInfo.guides.compactMap {
          try resolveIntArrayGuides($0)
        }
        return DynamicGenerationSchema.Property(
          name: propertyInfo.name,
          description: propertyInfo.description,
          schema: .init(type: [Int].self, guides: arrayGuides),
          isOptional: propertyInfo.isOptional
        )
      } else if typeName == "array<number>" {
        // Handle array of numbers
        let arrayGuides = try propertyInfo.guides.compactMap {
          try resolveDoubleArrayGuides($0)
        }
        return DynamicGenerationSchema.Property(
          name: propertyInfo.name,
          description: propertyInfo.description,
          schema: .init(type: [Double].self, guides: arrayGuides),
          isOptional: propertyInfo.isOptional
        )
      } else {
        // Handle referece array types like array<Person> where Person is a reference to some type
        let pattern = /array<(\w+)>/
        if let match = typeName.firstMatch(of: pattern) {
          let refTypeName = String(match.1)
          // A DynamicGenerationSchema reference will ensure the correct type is resolved
          let ref: DynamicGenerationSchema = DynamicGenerationSchema(
            referenceTo: refTypeName
          )
          // Only maximum and minimum are supported for arrays of references
          var maxCount: Int? = nil
          var minCount: Int? = nil
          try propertyInfo.guides.forEach { guide in
            switch guide {
            case .count(let count):
              maxCount = count
              minCount = count
            case .maxItems(let count):
              maxCount = count
            case .minItems(let count):
              minCount = count
            default:
              let context = LanguageModelSession.GenerationError.Context(
                debugDescription: "Unsupported guide for array of a referenced Generable type"
              )
              throw LanguageModelSession.GenerationError.unsupportedGuide(context)
            }
          }
          return DynamicGenerationSchema.Property(
            name: propertyInfo.name,
            schema: .init(arrayOf: ref, minimumElements: minCount, maximumElements: maxCount),
            isOptional: propertyInfo.isOptional
          )
        } else {
          // Fallback to array of strings
          let arrayGuides = try propertyInfo.guides.compactMap {
            try resolveArrayStringGuides($0)
          }
          return DynamicGenerationSchema.Property(
            name: propertyInfo.name,
            description: propertyInfo.description,
            schema: .init(type: [String].self, guides: arrayGuides),
            isOptional: propertyInfo.isOptional
          )
        }
      }
    case let typeName where !typeName.isEmpty:
      // Assume it's a reference to another schema
      return DynamicGenerationSchema.Property(
        name: propertyInfo.name,
        description: propertyInfo.description,
        schema: .init(referenceTo: typeName),
        isOptional: propertyInfo.isOptional
      )
    default:
      // Default to String with string-specific guides
      let stringGuides = try propertyInfo.guides.compactMap {
        try resolveStringGuides($0)

      }
      return DynamicGenerationSchema.Property(
        name: propertyInfo.name,
        description: propertyInfo.description,
        schema: .init(
          type: String.self,
          guides: stringGuides
        ),
        isOptional: propertyInfo.isOptional
      )
    }
  }
}

// MARK: - Tool implementation

final class BridgedTool: Tool {
  let name: String
  let description: String

  let id: Atomic<CUnsignedInt> = Atomic(0)

  let foreignCall: @convention(c) (FMGeneratedContentRef, CUnsignedInt) -> Void
  let outputContinuation = Mutex<[CUnsignedInt: CheckedContinuation<String, any Error>]>([:])
  let parameters: GenerationSchema

  init(
    name: String,
    description: String,
    parameters: GenerationSchema,
    foreignCall: @convention(c) (FMGeneratedContentRef, CUnsignedInt) -> Void
  ) {
    self.name = name
    self.description = description
    self.parameters = parameters
    self.foreignCall = foreignCall
  }

  func nextID() -> CUnsignedInt {
    id.wrappingAdd(1, ordering: .relaxed).newValue
  }

  func call(arguments: GeneratedContent) async throws -> String {
    let arguments = GeneratedContentWrapper(content: arguments)
    let id = nextID()
    foreignCall(FMGeneratedContentRef(Unmanaged.passRetained(arguments).toOpaque()), id)
    return try await withCheckedThrowingContinuation { continuation in
      outputContinuation.withLock {
        $0[id] = continuation
      }
    }
  }
}

@_cdecl("FMBridgedToolCreate")
public func FMBridgedToolCreate(
  name: UnsafePointer<CChar>,
  description: UnsafePointer<CChar>,
  parameters: FMGenerationSchemaRef,
  callable: @convention(c) (FMGeneratedContentRef, CUnsignedInt) -> Void,
  outErrorCode: UnsafeMutablePointer<Int32>?,
  outErrorDescription: UnsafeMutablePointer<UnsafePointer<CChar>?>?
) -> FMBridgedToolRef? {
  do {
    let schemaBuilder = Unmanaged<GenerationSchemaBuilder>.fromOpaque(parameters)
      .takeUnretainedValue()
    let schema = try schemaBuilder.buildSchema()
    let bridgedTool = BridgedTool(
      name: String(cString: name),
      description: String(cString: description),
      parameters: schema,
      foreignCall: callable
    )
    return FMBridgedToolRef(Unmanaged.passRetained(bridgedTool).toOpaque())
  } catch let error as LanguageModelSession.GenerationError {
    // Map specific generation errors to error codes
    let errorCode = mapGenerationErrorToStatusCode(error)
    let debugDescription = error.localizedDescription
    debugDescription.withCString { cString in
      outErrorCode?.pointee = errorCode
      outErrorDescription?.pointee = UnsafePointer(strdup(cString))
    }
    return nil
  } catch {
    // Generic error - unknown type
    let debugDescription = error.localizedDescription
    debugDescription.withCString { cString in
      outErrorCode?.pointee = StatusCode.unknownError.rawValue
      outErrorDescription?.pointee = UnsafePointer(strdup(cString))
    }
    return nil
  }
}

@_cdecl("FMBridgedToolFinishCall")
public func FMBridgedToolFinishCall(
  tool: FMBridgedToolRef,
  callId: CUnsignedInt,
  output: UnsafePointer<CChar>
) {
  let bridgedTool = Unmanaged<BridgedTool>.fromOpaque(tool).takeUnretainedValue()
  bridgedTool.outputContinuation.withLock {
    if let continuation = $0[callId] {
      continuation.resume(returning: String(cString: output))
      $0.removeValue(forKey: callId)
    }
  }
}
