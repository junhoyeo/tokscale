/*
For licensing see accompanying LICENSE file.
Copyright (C) 2026 Apple Inc. All Rights Reserved.
*/

import Testing
import FoundationModels
import FoundationModelsCDeclarations
import Synchronization

@Suite struct BasicSystemModelTests {
  @Test func testAvailability() async throws {
    let model = FMSystemLanguageModelGetDefault()
    var unavailableReason = FMSystemLanguageModelUnavailableReasonUnknown
    let isAvailable = FMSystemLanguageModelIsAvailable(model, &unavailableReason)
    switch SystemLanguageModel.default.availability {
    case .available:
      #expect(isAvailable)
    case .unavailable(.appleIntelligenceNotEnabled):
      #expect(!isAvailable)
      #expect(
        unavailableReason == FMSystemLanguageModelUnavailableReasonAppleIntelligenceNotEnabled
      )
    case .unavailable(.deviceNotEligible):
      #expect(!isAvailable)
      #expect(unavailableReason == FMSystemLanguageModelUnavailableReasonDeviceNotEligible)
    case .unavailable(.modelNotReady):
      #expect(!isAvailable)
      #expect(unavailableReason == FMSystemLanguageModelUnavailableReasonModelNotReady)
    @unknown default:
      #expect(!isAvailable)
      #expect(unavailableReason == FMSystemLanguageModelUnavailableReasonUnknown)
    }
    FMRelease(model)
  }

  @Test(.enabled(if: SystemLanguageModel.default.isAvailable))
  func testResponse() async throws {
    let model = FMSystemLanguageModelGetDefault()
    let session = FMLanguageModelSessionCreateFromSystemLanguageModel(
      model,
      "Your responses MUST be full of sarcasm.",
      nil,
      0
    )
    var isResponding: Bool = true
    FMLanguageModelSessionRespond(
      session,
      "What programming language is better, Swift or C?",
      &isResponding
    ) { status, content, length, userInfo in
      #expect(status == 0)
      let content = String(cString: try! #require(content))
      print(content)
      #expect(!content.isEmpty)
      #expect(strlen(content) == length)
      userInfo?.bindMemory(to: Bool.self, capacity: 1).pointee = false
    }
    while isResponding {}
    FMRelease(session)
    FMRelease(model)
  }

  @Test func testBridgedToolConcurrentCalls() async throws {
    // Test concurrent calls using a custom Tool implementation
    final class EchoTool: Tool, @unchecked Sendable {
      let name = "echo_tool"
      let description = "Echoes the input message"
      let callTracker = Mutex<[String]>([])

      let parameters: GenerationSchema = try! GenerationSchema(
        root: DynamicGenerationSchema(
          name: "EchoParams",
          description: "Parameters for echo tool",
          properties: [
            .init(
              name: "message",
              description: "The message to echo",
              schema: .init(type: String.self)
            )
          ]
        ),
        dependencies: []
      )

      func call(arguments: GeneratedContent) async throws -> String {
        let message: String = try arguments.value(forProperty: "message")

        callTracker.withLock { calls in
          calls.append(message)
        }

        // Simulate async work
        try? await Task.sleep(for: .milliseconds(10))

        return "Echo: \(message)"
      }

      func getCallCount() -> Int {
        callTracker.withLock { $0.count }
      }
    }

    let tool = EchoTool()
    let numberOfConcurrentCalls = 10

    await withTaskGroup(of: (Int, String).self) { group in
      // Launch concurrent calls
      for i in 0..<numberOfConcurrentCalls {
        group.addTask {
          let args = try! GeneratedContent(json: "{\"message\": \"test\(i)\"}")
          let result = try! await tool.call(arguments: args)
          return (i, result)
        }
      }

      // Collect results
      var results: [Int: String] = [:]
      for await (index, result) in group {
        results[index] = result
        print("Call \(index) completed: \(result)")
      }

      // Verify all calls completed successfully
      #expect(results.count == numberOfConcurrentCalls)

      // Verify each call received a unique response
      for i in 0..<numberOfConcurrentCalls {
        let result = try! #require(results[i])
        #expect(result.contains("test\(i)"))
        #expect(result.contains("Echo:"))
      }
    }

    // Verify the tool tracked all calls
    #expect(tool.getCallCount() == numberOfConcurrentCalls)
  }

  @Test func testBridgedToolSequentialCalls() async throws {
    // Test sequential calls using a custom Tool implementation
    final class CounterTool: Tool, @unchecked Sendable {
      let name = "counter_tool"
      let description = "Counts invocations"
      let callCount = Mutex<Int>(0)

      let parameters: GenerationSchema = try! GenerationSchema(
        root: DynamicGenerationSchema(
          name: "CountParams",
          description: "Parameters for counter tool",
          properties: [
            .init(name: "value", description: "The value to process", schema: .init(type: Int.self))
          ]
        ),
        dependencies: []
      )

      func call(arguments: GeneratedContent) async throws -> String {
        let currentCount = callCount.withLock { count in
          count += 1
          return count
        }

        return "Count: \(currentCount)"
      }

      func getCallCount() -> Int {
        callCount.withLock { $0 }
      }
    }

    let tool = CounterTool()

    // Make sequential calls to verify the tool can be reused
    for i in 1...5 {
      let args = try! GeneratedContent(json: "{\"value\": \(i)}")
      let result = try! await tool.call(arguments: args)
      print("Sequential call \(i): \(result)")
      #expect(result.contains("Count: \(i)"))
    }

    #expect(tool.getCallCount() == 5)
  }

  @Test func testBridgedToolUniqueIDGeneration() async throws {
    // Test the unique ID generation mechanism used by BridgedTool
    // We'll use Atomic directly since BridgedTool uses it internally
    let idGenerator = Atomic<CUnsignedInt>(0)
    let idTracker = Mutex<Set<CUnsignedInt>>([])

    let numberOfCalls = 50

    await withTaskGroup(of: Void.self) { group in
      for _ in 0..<numberOfCalls {
        group.addTask { @Sendable in
          // Simulate what BridgedTool.nextID() does
          let id = idGenerator.wrappingAdd(1, ordering: .relaxed).newValue

          idTracker.withLock { ids in
            _ = ids.insert(id)
          }
        }
      }
    }

    // All IDs should be unique
    let uniqueCount = idTracker.withLock { $0.count }
    #expect(uniqueCount == numberOfCalls)
    print("Generated \(uniqueCount) unique IDs out of \(numberOfCalls) calls")
  }
}
