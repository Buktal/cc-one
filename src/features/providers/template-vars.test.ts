import { describe, expect, it } from "vitest"
import { PROVIDER_PRESETS } from "@/features/providers/presets"
import {
  extractTemplateVars,
  replaceTemplateVarsInText,
  restoreTemplatePlaceholders,
} from "./template-vars"

describe("extractTemplateVars / replaceTemplateVarsInText", () => {
  it("collects the Bedrock preset's placeholder names, deduped, in order", () => {
    const bedrock = PROVIDER_PRESETS.find(
      (p) => p.name === "AWS Bedrock (AKSK)",
    )
    expect(bedrock).toBeDefined()
    expect(extractTemplateVars(bedrock!.settingsConfig)).toEqual([
      "AWS_REGION",
      "AWS_ACCESS_KEY_ID",
      "AWS_SECRET_ACCESS_KEY",
    ])
  })

  it("finds and replaces placeholders in nested non-env settings too", () => {
    const text = JSON.stringify({
      env: { ANTHROPIC_BASE_URL: "https://x.dev" },
      hooks: {
        PreToolUse: [
          {
            // biome-ignore lint/suspicious/noTemplateCurlyInString: 断言字面量占位符文本
            matcher: "http://${HOST}/cb",
          },
        ],
      },
    })
    expect(extractTemplateVars(text)).toEqual(["HOST"])
    const next = replaceTemplateVarsInText(text, { HOST: "127.0.0.1" })
    expect(JSON.parse(next)).toEqual({
      env: { ANTHROPIC_BASE_URL: "https://x.dev" },
      hooks: { PreToolUse: [{ matcher: "http://127.0.0.1/cb" }] },
    })
  })

  it("returns an empty list for a clean or garbage snapshot", () => {
    expect(extractTemplateVars("{}")).toEqual([])
    expect(extractTemplateVars("not-json")).toEqual([])
  })

  it("replaces filled variables and keeps the unfilled placeholders verbatim", () => {
    const bedrock = PROVIDER_PRESETS.find(
      (p) => p.name === "AWS Bedrock (AKSK)",
    )
    expect(bedrock).toBeDefined()
    const next = replaceTemplateVarsInText(bedrock!.settingsConfig, {
      AWS_REGION: "ap-northeast-1",
    })
    const env = (
      JSON.parse(next) as {
        env: Record<string, string>
      }
    ).env
    expect(env.ANTHROPIC_BASE_URL).toBe(
      "https://bedrock-runtime.ap-northeast-1.amazonaws.com",
    )
    expect(env.AWS_REGION).toBe("ap-northeast-1")
    expect(env.AWS_ACCESS_KEY_ID).toBe(
      // biome-ignore lint/suspicious/noTemplateCurlyInString: 断言字面量占位符文本
      "${AWS_ACCESS_KEY_ID}",
    )
    expect(extractTemplateVars(next)).toEqual([
      "AWS_ACCESS_KEY_ID",
      "AWS_SECRET_ACCESS_KEY",
    ])
  })

  it("keeps the placeholder verbatim when a variable is missing or empty", () => {
    const text = JSON.stringify({
      env: {
        // biome-ignore lint/suspicious/noTemplateCurlyInString: 断言字面量占位符文本
        ANTHROPIC_BASE_URL:
          "https://bedrock-runtime.${AWS_REGION}.amazonaws.com",
      },
    })
    expect(
      JSON.parse(replaceTemplateVarsInText(text, { AWS_REGION: "" })),
    ).toEqual(JSON.parse(text))
    expect(JSON.parse(replaceTemplateVarsInText(text, {}))).toEqual(
      JSON.parse(text),
    )
  })
})

describe("restoreTemplatePlaceholders", () => {
  it("reverts every occurrence of a recorded value to its placeholder", () => {
    const text = JSON.stringify({
      env: {
        ANTHROPIC_BASE_URL: "https://bedrock-runtime.us-east-1.amazonaws.com",
        AWS_REGION: "us-east-1",
      },
    })
    const restored = restoreTemplatePlaceholders(text, {
      AWS_REGION: "us-east-1",
    })
    expect(JSON.parse(restored)).toEqual({
      env: {
        // biome-ignore lint/suspicious/noTemplateCurlyInString: 断言字面量占位符文本
        ANTHROPIC_BASE_URL:
          "https://bedrock-runtime.${AWS_REGION}.amazonaws.com",
        // biome-ignore lint/suspicious/noTemplateCurlyInString: 断言字面量占位符文本
        AWS_REGION: "${AWS_REGION}",
      },
    })
  })

  it("leaves values without a recorded template untouched", () => {
    const text = JSON.stringify({
      env: { ANTHROPIC_BASE_URL: "https://x.dev" },
    })
    expect(
      JSON.parse(
        restoreTemplatePlaceholders(text, { AWS_REGION: "us-east-1" }),
      ),
    ).toEqual(JSON.parse(text))
  })
})

describe("Bedrock preset end to end", () => {
  const bedrock = PROVIDER_PRESETS.find((p) => p.name === "AWS Bedrock (AKSK)")!
  const values = {
    AWS_REGION: "ap-northeast-1",
    AWS_ACCESS_KEY_ID: "AKIAEXAMPLEKEY",
    AWS_SECRET_ACCESS_KEY: "SKEXAMPLE",
  }

  it("materializes the preset placeholders into the snapshot", () => {
    const next = replaceTemplateVarsInText(bedrock.settingsConfig, values)
    expect(extractTemplateVars(next)).toEqual([])
    const env = (
      JSON.parse(next) as {
        env: Record<string, string>
      }
    ).env
    expect(env.ANTHROPIC_BASE_URL).toBe(
      "https://bedrock-runtime.ap-northeast-1.amazonaws.com",
    )
    expect(env.AWS_REGION).toBe("ap-northeast-1")
    expect(env.AWS_ACCESS_KEY_ID).toBe("AKIAEXAMPLEKEY")
    expect(env.AWS_SECRET_ACCESS_KEY).toBe("SKEXAMPLE")
    // 非模板字段原样保留。
    expect(env.CLAUDE_CODE_USE_BEDROCK).toBe("1")
  })

  it("restores the placeholders verbatim for re-editing from meta values", () => {
    const materialized = replaceTemplateVarsInText(
      bedrock.settingsConfig,
      values,
    )
    expect(restoreTemplatePlaceholders(materialized, values)).toBe(
      bedrock.settingsConfig,
    )
  })
})
