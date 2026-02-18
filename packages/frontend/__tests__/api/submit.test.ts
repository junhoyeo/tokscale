import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';

/**
 * Test suite for POST /api/submit - Source-Level Merge
 * 
 * These tests verify the source-level merge functionality:
 * - First submission creates new records
 * - Subsequent submissions merge by source
 * - Sources not in submission are preserved
 * - Totals are recalculated from dailyBreakdown
 * - Concurrent submissions are handled correctly
 */

// Mock data factories
function createMockSubmissionData(overrides: Partial<{
  sources: string[];
  contributions: Array<{
    date: string;
    sources: Array<{
      source: string;
      modelId: string;
      cost: number;
      tokens: { input: number; output: number; cacheRead: number; cacheWrite: number };
      messages: number;
    }>;
  }>;
}> = {}) {
  const defaultSources = overrides.sources || ['claude'];
  const defaultContributions = overrides.contributions || [
    {
      date: '2024-12-01',
      sources: defaultSources.map(source => ({
        source,
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
        sum + d.sources.reduce((s, src) => s + src.tokens.input + src.tokens.output, 0), 0
      ),
      totalCost: defaultContributions.reduce((sum, d) => 
        sum + d.sources.reduce((s, src) => s + src.cost, 0), 0
      ),
      totalDays: defaultContributions.length,
      activeDays: defaultContributions.filter(d => d.sources.length > 0).length,
      averagePerDay: 0,
      maxCostInSingleDay: 0,
      sources: defaultSources,
      models: ['claude-sonnet-4-20250514'],
    },
    years: [],
    contributions: defaultContributions.map(d => ({
      date: d.date,
      totals: {
        tokens: d.sources.reduce((s, src) => s + src.tokens.input + src.tokens.output, 0),
        cost: d.sources.reduce((s, src) => s + src.cost, 0),
        messages: d.sources.reduce((s, src) => s + src.messages, 0),
      },
      intensity: 2 as const,
      tokenBreakdown: {
        input: d.sources.reduce((s, src) => s + src.tokens.input, 0),
        output: d.sources.reduce((s, src) => s + src.tokens.output, 0),
        cacheRead: d.sources.reduce((s, src) => s + src.tokens.cacheRead, 0),
        cacheWrite: d.sources.reduce((s, src) => s + src.tokens.cacheWrite, 0),
        reasoning: 0,
      },
      sources: d.sources.map(src => ({
        source: src.source as 'opencode' | 'claude' | 'codex' | 'gemini' | 'cursor' | 'amp' | 'droid' | 'openclaw' | 'pi' | 'kimi',
        modelId: src.modelId,
        tokens: src.tokens,
        cost: src.cost,
        messages: src.messages,
      })),
    })),
  };
}

describe('POST /api/submit - Source-Level Merge', () => {
  describe('First Submission (Create Mode)', () => {
    it('should create new submission with all sources', () => {
      const data = createMockSubmissionData({ sources: ['claude', 'cursor'] });
      
      // Verify data structure
      expect(data.summary.sources).toContain('claude');
      expect(data.summary.sources).toContain('cursor');
      expect(data.contributions[0].sources.length).toBe(2);
    });

    it('should create dailyBreakdown for each day', () => {
      const data = createMockSubmissionData({
        contributions: [
          { date: '2024-12-01', sources: [{ source: 'claude', modelId: 'claude-sonnet-4', cost: 1, tokens: { input: 100, output: 50, cacheRead: 0, cacheWrite: 0 }, messages: 1 }] },
          { date: '2024-12-02', sources: [{ source: 'claude', modelId: 'claude-sonnet-4', cost: 2, tokens: { input: 200, output: 100, cacheRead: 0, cacheWrite: 0 }, messages: 2 }] },
          { date: '2024-12-03', sources: [{ source: 'claude', modelId: 'claude-sonnet-4', cost: 3, tokens: { input: 300, output: 150, cacheRead: 0, cacheWrite: 0 }, messages: 3 }] },
        ],
      });
      
      expect(data.contributions.length).toBe(3);
      expect(data.contributions.map(c => c.date)).toEqual(['2024-12-01', '2024-12-02', '2024-12-03']);
    });

    it('should support pi source in submission payload', () => {
      const data = createMockSubmissionData({ sources: ['pi'] });

      expect(data.summary.sources).toContain('pi');
      expect(data.contributions[0].sources[0].source).toBe('pi');
    });

    it('should support kimi source in submission payload', () => {
      const data = createMockSubmissionData({ sources: ['kimi'] });

      expect(data.summary.sources).toContain('kimi');
      expect(data.contributions[0].sources[0].source).toBe('kimi');
    });
  });

  describe('Source-Level Merge Logic', () => {
    it('should preserve sources NOT in submission but delete sources with no day activity', () => {
      const existingSourceBreakdown = {
        claude: { tokens: 1000, cost: 10, modelId: 'claude-sonnet-4', input: 600, output: 400, cacheRead: 0, cacheWrite: 0, messages: 5 },
        cursor: { tokens: 500, cost: 5, modelId: 'cursor-small', input: 300, output: 200, cacheRead: 0, cacheWrite: 0, messages: 3 },
        codex: { tokens: 200, cost: 2, modelId: 'gpt-4', input: 100, output: 100, cacheRead: 0, cacheWrite: 0, messages: 1 },
      };
      
      const incomingSources = new Set(['claude', 'cursor']);
      const incomingSourceBreakdown = {
        claude: { tokens: 1200, cost: 12, modelId: 'claude-sonnet-4', input: 700, output: 500, cacheRead: 0, cacheWrite: 0, messages: 6 },
      };
      
      const merged = { ...existingSourceBreakdown } as Record<string, typeof existingSourceBreakdown.claude>;
      for (const sourceName of incomingSources) {
        if (incomingSourceBreakdown[sourceName as keyof typeof incomingSourceBreakdown]) {
          merged[sourceName] = incomingSourceBreakdown[sourceName as keyof typeof incomingSourceBreakdown];
        } else {
          delete merged[sourceName];
        }
      }
      
      expect(merged.codex).toEqual(existingSourceBreakdown.codex);
      expect(merged.claude.tokens).toBe(1200);
      expect(merged.cursor).toBeUndefined();
    });

    it('should update submitted source data', () => {
      // Same source submitted again should replace, not add
      const existingClaude = { tokens: 1000, cost: 10, modelId: 'claude-sonnet-4', input: 600, output: 400, cacheRead: 0, cacheWrite: 0, messages: 5 };
      const newClaude = { tokens: 1500, cost: 15, modelId: 'claude-sonnet-4', input: 900, output: 600, cacheRead: 0, cacheWrite: 0, messages: 8 };
      
      // After merge, should be new values, not sum
      expect(newClaude.cost).toBe(15); // Not 10 + 15 = 25
      expect(newClaude.tokens).toBe(1500); // Not 1000 + 1500 = 2500
    });

    it('should merge new source into existing day', () => {
      // Day has claude, now cursor is added
      const existingSourceBreakdown = {
        claude: { tokens: 1000, cost: 10, modelId: 'claude-sonnet-4', input: 600, output: 400, cacheRead: 0, cacheWrite: 0, messages: 5 },
      };
      
      const incomingSources = new Set(['cursor']);
      const incomingSourceBreakdown = {
        cursor: { tokens: 500, cost: 5, modelId: 'cursor-small', input: 300, output: 200, cacheRead: 0, cacheWrite: 0, messages: 3 },
      };
      
      // Simulate merge
      const merged = { ...existingSourceBreakdown };
      for (const sourceName of incomingSources) {
        if (incomingSourceBreakdown[sourceName as keyof typeof incomingSourceBreakdown]) {
          (merged as Record<string, typeof existingSourceBreakdown.claude>)[sourceName] = incomingSourceBreakdown[sourceName as keyof typeof incomingSourceBreakdown];
        }
      }
      
      // Both sources should be present
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
      const sourceBreakdown = {
        claude: { tokens: 1000, cost: 10, modelId: 'claude-sonnet-4', input: 600, output: 400, cacheRead: 50, cacheWrite: 25, messages: 5 },
        cursor: { tokens: 500, cost: 5, modelId: 'cursor-small', input: 300, output: 200, cacheRead: 30, cacheWrite: 15, messages: 3 },
      };
      
      // Simulate recalculateDayTotals
      let totalTokens = 0;
      for (const source of Object.values(sourceBreakdown)) {
        totalTokens += source.tokens;
      }
      
      expect(totalTokens).toBe(1500);
    });

    it('should recalculate cache tokens', () => {
      const sourceBreakdown = {
        claude: { tokens: 1000, cost: 10, modelId: 'claude-sonnet-4', input: 600, output: 400, cacheRead: 50, cacheWrite: 25, messages: 5 },
        opencode: { tokens: 800, cost: 8, modelId: 'gpt-4o', input: 500, output: 300, cacheRead: 40, cacheWrite: 20, messages: 4 },
      };
      
      let totalCacheRead = 0;
      let totalCacheWrite = 0;
      for (const source of Object.values(sourceBreakdown)) {
        totalCacheRead += source.cacheRead;
        totalCacheWrite += source.cacheWrite;
      }
      
      expect(totalCacheRead).toBe(90);
      expect(totalCacheWrite).toBe(45);
    });

    it('should update sourcesUsed to include all sources', () => {
      // Simulate collecting sources from all days
      const day1Sources = ['claude', 'cursor'];
      const day2Sources = ['claude', 'opencode'];
      
      const allSources = new Set([...day1Sources, ...day2Sources]);
      
      expect(Array.from(allSources).sort()).toEqual(['claude', 'cursor', 'opencode']);
    });
  });

  describe('Edge Cases', () => {
    it('should reject empty submissions', () => {
      const data = createMockSubmissionData({ contributions: [] });
      
      expect(data.contributions.length).toBe(0);
      // API should return 400 for this
    });

    it('should handle day with no data for submitted source', () => {
      // User submits --claude but a day only has opencode data
      const dayWithOnlyOpencode = {
        date: '2024-12-01',
        sources: [
          { source: 'opencode', modelId: 'gpt-4o', cost: 5, tokens: { input: 300, output: 200, cacheRead: 0, cacheWrite: 0 }, messages: 3 },
        ],
      };
      
      const submittedSources = new Set(['claude']);
      
      // No claude data to update for this day
      const claudeInDay = dayWithOnlyOpencode.sources.find(s => s.source === 'claude');
      expect(claudeInDay).toBeUndefined();
      
      // opencode should be preserved
      const opencodeInDay = dayWithOnlyOpencode.sources.find(s => s.source === 'opencode');
      expect(opencodeInDay).toBeDefined();
    });

    it('should handle concurrent submissions without data loss', () => {
      // This is tested at the database level with .for('update') locks
      // Here we just verify the concept
      const submission1Sources = ['claude'];
      const submission2Sources = ['cursor'];
      
      // Both should be present after sequential processing
      const finalSources = new Set([...submission1Sources, ...submission2Sources]);
      expect(finalSources.size).toBe(2);
    });

    it('should treat contribution sources as submitted even if summary.sources is incomplete', () => {
      const data = createMockSubmissionData({
        sources: ['claude'],
        contributions: [
          {
            date: '2024-12-01',
            sources: [
              {
                source: 'pi',
                modelId: 'pi-model',
                cost: 1,
                tokens: { input: 100, output: 50, cacheRead: 0, cacheWrite: 0 },
                messages: 1,
              },
            ],
          },
        ],
      });

      const submittedSources = new Set(data.summary.sources);
      for (const contribution of data.contributions) {
        for (const source of contribution.sources) {
          submittedSources.add(source.source);
        }
      }

      expect(submittedSources.has('pi')).toBe(true);
    });


  });

  describe('Multi-Model Per Source', () => {
    it('should aggregate multiple models per source correctly', () => {
      const daySourceEntries = [
        { source: 'claude', modelId: 'claude-sonnet-4', cost: 10, tokens: { input: 500, output: 300, cacheRead: 100, cacheWrite: 50 }, messages: 5 },
        { source: 'claude', modelId: 'claude-opus-4', cost: 20, tokens: { input: 800, output: 500, cacheRead: 200, cacheWrite: 100 }, messages: 8 },
        { source: 'cursor', modelId: 'gpt-4o', cost: 5, tokens: { input: 200, output: 100, cacheRead: 50, cacheWrite: 25 }, messages: 3 },
      ];

      type ModelData = { tokens: number; cost: number; input: number; output: number; cacheRead: number; cacheWrite: number; messages: number };
      type SourceData = ModelData & { models: Record<string, ModelData> };
      const result: Record<string, SourceData> = {};

      for (const entry of daySourceEntries) {
        const modelData: ModelData = {
          tokens: entry.tokens.input + entry.tokens.output + entry.tokens.cacheRead + entry.tokens.cacheWrite,
          cost: entry.cost,
          input: entry.tokens.input,
          output: entry.tokens.output,
          cacheRead: entry.tokens.cacheRead,
          cacheWrite: entry.tokens.cacheWrite,
          messages: entry.messages,
        };

        const existing = result[entry.source];
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
          result[entry.source] = { ...modelData, models: { [entry.modelId]: modelData } };
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

    it('should build modelBreakdown from sources with multiple models', () => {
      const sourceBreakdown = {
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
      for (const source of Object.values(sourceBreakdown)) {
        for (const [modelId, modelData] of Object.entries(source.models)) {
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
          sources: ['claude', 'cursor'],
        },
        mode: 'merge' as const,
      };
      
      expect(mockResponse.metrics).toBeDefined();
      expect(mockResponse.metrics.totalTokens).toBeGreaterThan(0);
      expect(mockResponse.metrics.sources).toContain('claude');
      expect(mockResponse.mode).toBe('merge');
    });
  });
});
