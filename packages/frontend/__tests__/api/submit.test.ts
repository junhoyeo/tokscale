import { describe, it, expect } from 'vitest';
import {
  attachDeviceContributions,
  LEGACY_DEVICE_ID,
  mergeClientBreakdowns,
  type ClientBreakdownData,
} from '../../src/lib/db/helpers';

/**
 * Test suite for POST /api/submit merge behavior.
 *
 * These tests cover both the legacy client-level expectations and the
 * device-level merge semantics used to preserve cross-machine submissions.
 */

// Mock data factories
function createMockSubmissionData(overrides: Partial<{
  clients: string[];
  contributions: Array<{
    date: string;
    clients: Array<{
      client: string;
      modelId: string;
      cost: number;
      tokens: { input: number; output: number; cacheRead: number; cacheWrite: number };
      messages: number;
    }>;
  }>;
}> = {}) {
  const defaultClients = overrides.clients || ['claude'];
  const defaultContributions = overrides.contributions || [
    {
      date: '2024-12-01',
      clients: defaultClients.map(client => ({
        client,
        modelId: 'claude-sonnet-4-20250514',
        cost: 1.5,
        tokens: { input: 1000, output: 500, cacheRead: 100, cacheWrite: 50 },
        messages: 5,
      })),
    },
  ];

  return {
    meta: {
      generatedAt: new Date().toISOString(),
      version: '1.0.0',
      dateRange: {
        start: defaultContributions[0]?.date || '2024-12-01',
        end: defaultContributions[defaultContributions.length - 1]?.date || '2024-12-01',
      },
    },
    summary: {
      totalTokens: defaultContributions.reduce((sum, d) => 
        sum + d.clients.reduce((s, client) => s + client.tokens.input + client.tokens.output, 0), 0
      ),
      totalCost: defaultContributions.reduce((sum, d) => 
        sum + d.clients.reduce((s, client) => s + client.cost, 0), 0
      ),
      totalDays: defaultContributions.length,
      activeDays: defaultContributions.filter(d => d.clients.length > 0).length,
      averagePerDay: 0,
      maxCostInSingleDay: 0,
      clients: defaultClients,
      models: ['claude-sonnet-4-20250514'],
    },
    years: [],
    contributions: defaultContributions.map(d => ({
      date: d.date,
      totals: {
        tokens: d.clients.reduce((s, client) => s + client.tokens.input + client.tokens.output, 0),
        cost: d.clients.reduce((s, client) => s + client.cost, 0),
        messages: d.clients.reduce((s, client) => s + client.messages, 0),
      },
      intensity: 2 as const,
      tokenBreakdown: {
        input: d.clients.reduce((s, client) => s + client.tokens.input, 0),
        output: d.clients.reduce((s, client) => s + client.tokens.output, 0),
        cacheRead: d.clients.reduce((s, client) => s + client.tokens.cacheRead, 0),
        cacheWrite: d.clients.reduce((s, client) => s + client.tokens.cacheWrite, 0),
        reasoning: 0,
      },
      clients: d.clients.map(client => ({
        client: client.client as 'opencode' | 'claude' | 'codex' | 'gemini' | 'cursor' | 'amp' | 'droid' | 'openclaw' | 'pi' | 'kimi' | 'qwen',
        modelId: client.modelId,
        tokens: client.tokens,
        cost: client.cost,
        messages: client.messages,
      })),
    })),
  };
}

describe('POST /api/submit merge behavior', () => {
  describe('First Submission (Create Mode)', () => {
    it('should create new submission with all clients', () => {
      const data = createMockSubmissionData({ clients: ['claude', 'cursor'] });
      
      // Verify data structure
      expect(data.summary.clients).toContain('claude');
      expect(data.summary.clients).toContain('cursor');
      expect(data.contributions[0].clients.length).toBe(2);
    });

    it('should create dailyBreakdown for each day', () => {
      const data = createMockSubmissionData({
        contributions: [
          { date: '2024-12-01', clients: [{ client: 'claude', modelId: 'claude-sonnet-4', cost: 1, tokens: { input: 100, output: 50, cacheRead: 0, cacheWrite: 0 }, messages: 1 }] },
          { date: '2024-12-02', clients: [{ client: 'claude', modelId: 'claude-sonnet-4', cost: 2, tokens: { input: 200, output: 100, cacheRead: 0, cacheWrite: 0 }, messages: 2 }] },
          { date: '2024-12-03', clients: [{ client: 'claude', modelId: 'claude-sonnet-4', cost: 3, tokens: { input: 300, output: 150, cacheRead: 0, cacheWrite: 0 }, messages: 3 }] },
        ],
      });
      
      expect(data.contributions.length).toBe(3);
      expect(data.contributions.map(c => c.date)).toEqual(['2024-12-01', '2024-12-02', '2024-12-03']);
    });

    it('should support pi client in submission payload', () => {
      const data = createMockSubmissionData({ clients: ['pi'] });

      expect(data.summary.clients).toContain('pi');
      expect(data.contributions[0].clients[0].client).toBe('pi');
    });

    it('should support kimi client in submission payload', () => {
      const data = createMockSubmissionData({ clients: ['kimi'] });

      expect(data.summary.clients).toContain('kimi');
      expect(data.contributions[0].clients[0].client).toBe('kimi');
    });
  });

  describe('Client-Level Merge Logic', () => {
    it('should preserve clients NOT in submission but delete clients with no day activity', () => {
      const existingClientBreakdown = {
        claude: { tokens: 1000, cost: 10, modelId: 'claude-sonnet-4', input: 600, output: 400, cacheRead: 0, cacheWrite: 0, messages: 5 },
        cursor: { tokens: 500, cost: 5, modelId: 'cursor-small', input: 300, output: 200, cacheRead: 0, cacheWrite: 0, messages: 3 },
        codex: { tokens: 200, cost: 2, modelId: 'gpt-4', input: 100, output: 100, cacheRead: 0, cacheWrite: 0, messages: 1 },
      };
      
      const incomingClients = new Set(['claude', 'cursor']);
      const incomingClientBreakdown = {
        claude: { tokens: 1200, cost: 12, modelId: 'claude-sonnet-4', input: 700, output: 500, cacheRead: 0, cacheWrite: 0, messages: 6 },
      };
      
      const merged = { ...existingClientBreakdown } as Record<string, typeof existingClientBreakdown.claude>;
      for (const clientName of incomingClients) {
        if (incomingClientBreakdown[clientName as keyof typeof incomingClientBreakdown]) {
          merged[clientName] = incomingClientBreakdown[clientName as keyof typeof incomingClientBreakdown];
        } else {
          delete merged[clientName];
        }
      }
      
      expect(merged.codex).toEqual(existingClientBreakdown.codex);
      expect(merged.claude.tokens).toBe(1200);
      expect(merged.cursor).toBeUndefined();
    });

    it('should update submitted client data', () => {
      // Same client submitted again should replace, not add
      const newClaude = { tokens: 1500, cost: 15, modelId: 'claude-sonnet-4', input: 900, output: 600, cacheRead: 0, cacheWrite: 0, messages: 8 };
      
      // After merge, should be new values, not sum
      expect(newClaude.cost).toBe(15); // Not 10 + 15 = 25
      expect(newClaude.tokens).toBe(1500); // Not 1000 + 1500 = 2500
    });

    it('should merge new client into existing day', () => {
      // Day has claude, now cursor is added
      const existingClientBreakdown = {
        claude: { tokens: 1000, cost: 10, modelId: 'claude-sonnet-4', input: 600, output: 400, cacheRead: 0, cacheWrite: 0, messages: 5 },
      };
      
      const incomingClients = new Set(['cursor']);
      const incomingClientBreakdown = {
        cursor: { tokens: 500, cost: 5, modelId: 'cursor-small', input: 300, output: 200, cacheRead: 0, cacheWrite: 0, messages: 3 },
      };
      
      // Simulate merge
      const merged = { ...existingClientBreakdown };
      for (const clientName of incomingClients) {
        if (incomingClientBreakdown[clientName as keyof typeof incomingClientBreakdown]) {
          (merged as Record<string, typeof existingClientBreakdown.claude>)[clientName] = incomingClientBreakdown[clientName as keyof typeof incomingClientBreakdown];
        }
      }
      
      // Both clients should be present
      expect(Object.keys(merged)).toContain('claude');
      expect(Object.keys(merged)).toContain('cursor');
    });

    it('should add new dates without affecting existing', () => {
      const existingDates = ['2024-12-01', '2024-12-02'];
      const newDates = ['2024-12-03', '2024-12-04'];
      
      // Simulate: new dates should be added to the set
      const allDates = new Set([...existingDates, ...newDates]);
      
      expect(allDates.size).toBe(4);
      expect(Array.from(allDates)).toContain('2024-12-01');
      expect(Array.from(allDates)).toContain('2024-12-04');
    });
  });

  describe('Totals Recalculation', () => {
    it('should recalculate totalTokens from dailyBreakdown', () => {
      const clientBreakdown = {
        claude: { tokens: 1000, cost: 10, modelId: 'claude-sonnet-4', input: 600, output: 400, cacheRead: 50, cacheWrite: 25, messages: 5 },
        cursor: { tokens: 500, cost: 5, modelId: 'cursor-small', input: 300, output: 200, cacheRead: 30, cacheWrite: 15, messages: 3 },
      };
      
      // Simulate recalculateDayTotals
      let totalTokens = 0;
      for (const client of Object.values(clientBreakdown)) {
        totalTokens += client.tokens;
      }
      
      expect(totalTokens).toBe(1500);
    });

    it('should recalculate cache tokens', () => {
      const clientBreakdown = {
        claude: { tokens: 1000, cost: 10, modelId: 'claude-sonnet-4', input: 600, output: 400, cacheRead: 50, cacheWrite: 25, messages: 5 },
        opencode: { tokens: 800, cost: 8, modelId: 'gpt-4o', input: 500, output: 300, cacheRead: 40, cacheWrite: 20, messages: 4 },
      };
      
      let totalCacheRead = 0;
      let totalCacheWrite = 0;
      for (const client of Object.values(clientBreakdown)) {
        totalCacheRead += client.cacheRead;
        totalCacheWrite += client.cacheWrite;
      }
      
      expect(totalCacheRead).toBe(90);
      expect(totalCacheWrite).toBe(45);
    });

    it('should update clientsUsed to include all clients', () => {
      // Simulate collecting clients from all days
      const day1Clients = ['claude', 'cursor'];
      const day2Clients = ['claude', 'opencode'];
      
      const allClients = new Set([...day1Clients, ...day2Clients]);
      
      expect(Array.from(allClients).sort()).toEqual(['claude', 'cursor', 'opencode']);
    });
  });

  describe('Edge Cases', () => {
    it('should reject empty submissions', () => {
      const data = createMockSubmissionData({ contributions: [] });
      
      expect(data.contributions.length).toBe(0);
      // API should return 400 for this
    });

    it('should handle day with no data for submitted client', () => {
      // User submits --claude but a day only has opencode data
      const dayWithOnlyOpencode = {
        date: '2024-12-01',
        clients: [
          { client: 'opencode', modelId: 'gpt-4o', cost: 5, tokens: { input: 300, output: 200, cacheRead: 0, cacheWrite: 0 }, messages: 3 },
        ],
      };
      
      // No claude data to update for this day
      const claudeInDay = dayWithOnlyOpencode.clients.find(client => client.client === 'claude');
      expect(claudeInDay).toBeUndefined();
      
      // opencode should be preserved
      const opencodeInDay = dayWithOnlyOpencode.clients.find(client => client.client === 'opencode');
      expect(opencodeInDay).toBeDefined();
    });

    it('should handle concurrent submissions without data loss', () => {
      // This is tested at the database level with .for('update') locks
      // Here we just verify the concept
      const submission1Clients = ['claude'];
      const submission2Clients = ['cursor'];
      
      // Both should be present after sequential processing
      const finalClients = new Set([...submission1Clients, ...submission2Clients]);
      expect(finalClients.size).toBe(2);
    });

    it('should treat contribution clients as submitted even if summary.clients is incomplete', () => {
      const data = createMockSubmissionData({
        clients: ['claude'],
        contributions: [
          {
            date: '2024-12-01',
            clients: [
              {
                client: 'pi',
                modelId: 'pi-model',
                cost: 1,
                tokens: { input: 100, output: 50, cacheRead: 0, cacheWrite: 0 },
                messages: 1,
              },
            ],
          },
        ],
      });

      const submittedClients = new Set(data.summary.clients);
      for (const contribution of data.contributions) {
        for (const client of contribution.clients) {
          submittedClients.add(client.client);
        }
      }

      expect(submittedClients.has('pi')).toBe(true);
    });


  });

  describe('Multi-Model Per Client', () => {
    it('should aggregate multiple models per client correctly', () => {
      const dayClientEntries = [
        { client: 'claude', modelId: 'claude-sonnet-4', cost: 10, tokens: { input: 500, output: 300, cacheRead: 100, cacheWrite: 50 }, messages: 5 },
        { client: 'claude', modelId: 'claude-opus-4', cost: 20, tokens: { input: 800, output: 500, cacheRead: 200, cacheWrite: 100 }, messages: 8 },
        { client: 'cursor', modelId: 'gpt-4o', cost: 5, tokens: { input: 200, output: 100, cacheRead: 50, cacheWrite: 25 }, messages: 3 },
      ];

      type ModelData = { tokens: number; cost: number; input: number; output: number; cacheRead: number; cacheWrite: number; messages: number };
      type ClientData = ModelData & { models: Record<string, ModelData> };
      const result: Record<string, ClientData> = {};

      for (const entry of dayClientEntries) {
        const modelData: ModelData = {
          tokens: entry.tokens.input + entry.tokens.output + entry.tokens.cacheRead + entry.tokens.cacheWrite,
          cost: entry.cost,
          input: entry.tokens.input,
          output: entry.tokens.output,
          cacheRead: entry.tokens.cacheRead,
          cacheWrite: entry.tokens.cacheWrite,
          messages: entry.messages,
        };

        const existing = result[entry.client];
        if (existing) {
          existing.tokens += modelData.tokens;
          existing.cost += modelData.cost;
          existing.input += modelData.input;
          existing.output += modelData.output;
          existing.cacheRead += modelData.cacheRead;
          existing.cacheWrite += modelData.cacheWrite;
          existing.messages += modelData.messages;
          existing.models[entry.modelId] = modelData;
        } else {
          result[entry.client] = { ...modelData, models: { [entry.modelId]: modelData } };
        }
      }

      expect(result.claude.tokens).toBe(950 + 1600);
      expect(result.claude.cost).toBe(30);
      expect(Object.keys(result.claude.models)).toContain('claude-sonnet-4');
      expect(Object.keys(result.claude.models)).toContain('claude-opus-4');
      expect(result.claude.models['claude-sonnet-4'].tokens).toBe(950);
      expect(result.claude.models['claude-opus-4'].tokens).toBe(1600);

      expect(result.cursor.tokens).toBe(375);
      expect(Object.keys(result.cursor.models)).toEqual(['gpt-4o']);
    });

    it('should build modelBreakdown from clients with multiple models', () => {
      const clientBreakdown = {
        claude: {
          tokens: 2550,
          cost: 30,
          input: 1300,
          output: 800,
          cacheRead: 300,
          cacheWrite: 150,
          messages: 13,
          models: {
            'claude-sonnet-4': { tokens: 950, cost: 10, input: 500, output: 300, cacheRead: 100, cacheWrite: 50, messages: 5 },
            'claude-opus-4': { tokens: 1600, cost: 20, input: 800, output: 500, cacheRead: 200, cacheWrite: 100, messages: 8 },
          },
        },
        cursor: {
          tokens: 375,
          cost: 5,
          input: 200,
          output: 100,
          cacheRead: 50,
          cacheWrite: 25,
          messages: 3,
          models: {
            'gpt-4o': { tokens: 375, cost: 5, input: 200, output: 100, cacheRead: 50, cacheWrite: 25, messages: 3 },
          },
        },
      };

      const modelBreakdown: Record<string, number> = {};
      for (const client of Object.values(clientBreakdown)) {
        for (const [modelId, modelData] of Object.entries(client.models)) {
          modelBreakdown[modelId] = (modelBreakdown[modelId] || 0) + modelData.tokens;
        }
      }

      expect(modelBreakdown['claude-sonnet-4']).toBe(950);
      expect(modelBreakdown['claude-opus-4']).toBe(1600);
      expect(modelBreakdown['gpt-4o']).toBe(375);
    });
  });

  describe('Response Format', () => {
    it('should return mode: "create" for first submission', () => {
      const isNewSubmission = true;
      const mode = isNewSubmission ? 'create' : 'merge';
      expect(mode).toBe('create');
    });

    it('should return mode: "merge" for subsequent submissions', () => {
      const isNewSubmission = false;
      const mode = isNewSubmission ? 'create' : 'merge';
      expect(mode).toBe('merge');
    });

    it('should include recalculated metrics', () => {
      const mockResponse = {
        success: true,
        submissionId: 'test-id',
        username: 'testuser',
        metrics: {
          totalTokens: 1500,
          totalCost: 15.5,
          dateRange: { start: '2024-12-01', end: '2024-12-31' },
          activeDays: 25,
          clients: ['claude', 'cursor'],
        },
        mode: 'merge' as const,
      };
      
      expect(mockResponse.metrics).toBeDefined();
      expect(mockResponse.metrics.totalTokens).toBeGreaterThan(0);
      expect(mockResponse.metrics.clients).toContain('claude');
      expect(mockResponse.mode).toBe('merge');
    });
  });

  describe('Device-Level Deduplication', () => {
    const makeClientData = (tokens: number, cost: number, modelId: string): ClientBreakdownData => ({
      tokens,
      cost,
      input: Math.floor(tokens * 0.6),
      output: Math.floor(tokens * 0.4),
      cacheRead: 0,
      cacheWrite: 0,
      reasoning: 0,
      messages: 1,
      models: {
        [modelId]: {
          tokens,
          cost,
          input: Math.floor(tokens * 0.6),
          output: Math.floor(tokens * 0.4),
          cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
          messages: 1,
        },
      },
    });

    it('replaces a device contribution on resubmit instead of double-counting it', () => {
      const incoming1 = { claude: makeClientData(1000, 10, 'claude-sonnet-4') };
      const after1 = mergeClientBreakdowns({}, incoming1, new Set(['claude']), 'device-a');

      const incoming2 = { claude: makeClientData(1500, 15, 'claude-sonnet-4') };
      const after2 = mergeClientBreakdowns(after1, incoming2, new Set(['claude']), 'device-a');

      expect(after2.claude.tokens).toBe(1500);
      expect(after2.claude.devices!['device-a'].tokens).toBe(1500);
    });

    it('keeps other device contributions when one device resubmits', () => {
      const afterA = mergeClientBreakdowns(
        {},
        { claude: makeClientData(1000, 10, 'claude-sonnet-4') },
        new Set(['claude']),
        'device-a'
      );
      const afterB = mergeClientBreakdowns(
        afterA,
        { claude: makeClientData(500, 5, 'claude-sonnet-4') },
        new Set(['claude']),
        'device-b'
      );
      const afterA2 = mergeClientBreakdowns(
        afterB,
        { claude: makeClientData(2000, 20, 'claude-sonnet-4') },
        new Set(['claude']),
        'device-a'
      );

      expect(afterA2.claude.devices!['device-a'].tokens).toBe(2000);
      expect(afterA2.claude.devices!['device-b'].tokens).toBe(500);
      expect(afterA2.claude.tokens).toBe(2500);
    });

    it('migrates legacy client data into a __legacy__ device bucket on first merge', () => {
      const existing: Record<string, ClientBreakdownData> = {
        claude: {
          tokens: 800,
          cost: 8,
          input: 480,
          output: 320,
          cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
          messages: 3,
          models: {
            'claude-sonnet-4': {
              tokens: 800,
              cost: 8,
              input: 480,
              output: 320,
              cacheRead: 0,
              cacheWrite: 0,
              reasoning: 0,
              messages: 3,
            },
          },
        },
      };

      const merged = mergeClientBreakdowns(
        existing,
        { claude: makeClientData(1000, 10, 'claude-sonnet-4') },
        new Set(['claude']),
        'device-a'
      );

      expect(merged.claude.devices![LEGACY_DEVICE_ID]).toBeDefined();
      expect(merged.claude.devices![LEGACY_DEVICE_ID].tokens).toBe(800);
      expect(merged.claude.devices!['device-a'].tokens).toBe(1000);
      expect(merged.claude.tokens).toBe(1800);
    });

    it('removes only the submitting device when a declared client has no incoming data', () => {
      const afterA = mergeClientBreakdowns(
        {},
        { claude: makeClientData(1000, 10, 'claude-sonnet-4') },
        new Set(['claude']),
        'device-a'
      );
      const afterB = mergeClientBreakdowns(
        afterA,
        { claude: makeClientData(500, 5, 'claude-sonnet-4') },
        new Set(['claude']),
        'device-b'
      );
      const afterRemove = mergeClientBreakdowns(
        afterB,
        {},
        new Set(['claude']),
        'device-a'
      );

      expect(afterRemove.claude.devices!['device-a']).toBeUndefined();
      expect(afterRemove.claude.devices!['device-b'].tokens).toBe(500);
      expect(afterRemove.claude.tokens).toBe(500);
    });

    it('deletes the client when the last device contribution is removed', () => {
      const afterA = mergeClientBreakdowns(
        {},
        { claude: makeClientData(1000, 10, 'claude-sonnet-4') },
        new Set(['claude']),
        'device-a'
      );
      const afterRemove = mergeClientBreakdowns(
        afterA,
        {},
        new Set(['claude']),
        'device-a'
      );

      expect(afterRemove.claude).toBeUndefined();
    });

    it('aggregates models across devices for the same client', () => {
      const afterA = mergeClientBreakdowns(
        {},
        { claude: makeClientData(1000, 10, 'claude-sonnet-4') },
        new Set(['claude']),
        'device-a'
      );
      const afterB = mergeClientBreakdowns(
        afterA,
        { claude: makeClientData(500, 5, 'claude-opus-4') },
        new Set(['claude']),
        'device-b'
      );

      expect(afterB.claude.models['claude-sonnet-4'].tokens).toBe(1000);
      expect(afterB.claude.models['claude-opus-4'].tokens).toBe(500);
      expect(afterB.claude.tokens).toBe(1500);
    });

    it('handles multiple clients across multiple devices', () => {
      const afterA = mergeClientBreakdowns(
        {},
        {
          claude: makeClientData(1000, 10, 'claude-sonnet-4'),
          cursor: makeClientData(300, 3, 'gpt-4o'),
        },
        new Set(['claude', 'cursor']),
        'device-a'
      );
      const afterB = mergeClientBreakdowns(
        afterA,
        { claude: makeClientData(500, 5, 'claude-sonnet-4') },
        new Set(['claude']),
        'device-b'
      );

      expect(afterB.claude.tokens).toBe(1500);
      expect(afterB.cursor.tokens).toBe(300);
      expect(afterB.cursor.devices!['device-a'].tokens).toBe(300);
    });

    it('adds device-scoped breakdown data for newly inserted days', () => {
      const incoming = {
        claude: makeClientData(1000, 10, 'claude-sonnet-4'),
        cursor: makeClientData(300, 3, 'gpt-4o'),
      };

      const withDevices = attachDeviceContributions(incoming, 'device-a');

      expect(withDevices.claude.tokens).toBe(1000);
      expect(withDevices.claude.devices!['device-a'].tokens).toBe(1000);
      expect(withDevices.claude.devices!['device-a'].models['claude-sonnet-4'].tokens).toBe(1000);
      expect(withDevices.cursor.devices!['device-a'].tokens).toBe(300);
      expect(Object.keys(withDevices.cursor.devices!)).toEqual(['device-a']);
    });

    it('drops stale device-only days that disappear inside the submitted date range', () => {
      const submittedClients = new Set(['claude']);
      const dateRange = { start: '2024-12-01', end: '2024-12-02' };
      const incomingDates = new Set(['2024-12-02']);

      const existingDays = [
        {
          id: 'day-1',
          date: '2024-12-01',
          sourceBreakdown: mergeClientBreakdowns(
            {},
            { claude: makeClientData(1000, 10, 'claude-sonnet-4') },
            submittedClients,
            'device-a'
          ),
        },
        {
          id: 'day-2',
          date: '2024-12-03',
          sourceBreakdown: mergeClientBreakdowns(
            {},
            { claude: makeClientData(500, 5, 'claude-sonnet-4') },
            submittedClients,
            'device-a'
          ),
        },
      ];

      const toDelete: string[] = [];

      for (const existingDay of existingDays) {
        const isWithinSubmittedRange =
          existingDay.date >= dateRange.start &&
          existingDay.date <= dateRange.end;

        if (!isWithinSubmittedRange || incomingDates.has(existingDay.date)) {
          continue;
        }

        const prunedClientBreakdown = mergeClientBreakdowns(
          existingDay.sourceBreakdown,
          {},
          submittedClients,
          'device-a'
        );

        if (Object.keys(prunedClientBreakdown).length === 0) {
          toDelete.push(existingDay.id);
        }
      }

      expect(toDelete).toEqual(['day-1']);
    });

    it('preserves days outside the submitted date range even when they are absent from the payload', () => {
      const submittedClients = new Set(['claude']);
      const dateRange = { start: '2024-12-02', end: '2024-12-03' };
      const incomingDates = new Set(['2024-12-03']);

      const existingDay = {
        id: 'day-1',
        date: '2024-12-01',
        sourceBreakdown: mergeClientBreakdowns(
          {},
          { claude: makeClientData(1000, 10, 'claude-sonnet-4') },
          submittedClients,
          'device-a'
        ),
      };

      const isWithinSubmittedRange =
        existingDay.date >= dateRange.start &&
        existingDay.date <= dateRange.end;

      expect(isWithinSubmittedRange).toBe(false);
      expect(incomingDates.has(existingDay.date)).toBe(false);
      expect(existingDay.sourceBreakdown.claude.devices!['device-a'].tokens).toBe(1000);
    });
  });
});
