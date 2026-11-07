document.addEventListener("DOMContentLoaded", function () {
  const pageContainer = document.getElementById("deck-proposal-rules-page");
  if (!pageContainer) return;

  const deckHash = pageContainer.dataset.deckHash;
  const form = document.getElementById("deckProposalRulesForm");
  const saveBtn = document.getElementById("saveRulesBtn");
  const statusSpan = document.getElementById("rulesSaveStatus");

  // Dynamic toggle bindings
  const autoTagNewCheckbox = document.getElementById("auto_tag_new_cards");
  const newCardPatternInput = document.getElementById("new_card_tag_pattern");
  const autoTagUpdatedCheckbox = document.getElementById("auto_tag_updated_cards");
  const updatedCardPatternInput = document.getElementById("updated_card_tag_pattern");
  const pruneOldCheckbox = document.getElementById("prune_old_update_tags");
  const oldUpdatePatternInput = document.getElementById("old_update_tag_pattern");
  const enableMediaCheckbox = document.getElementById("enable_media_compression");
  const mediaControlsGroup = document.getElementById("media-controls-group");

  function updateDependencyStates() {
    if (newCardPatternInput && autoTagNewCheckbox) {
      newCardPatternInput.disabled = !autoTagNewCheckbox.checked;
    }
    if (updatedCardPatternInput && autoTagUpdatedCheckbox) {
      updatedCardPatternInput.disabled = !autoTagUpdatedCheckbox.checked;
    }
    if (oldUpdatePatternInput && pruneOldCheckbox && autoTagUpdatedCheckbox) {
      oldUpdatePatternInput.disabled = !autoTagUpdatedCheckbox.checked || !pruneOldCheckbox.checked;
    }
    if (enableMediaCheckbox && mediaControlsGroup) {
      const active = enableMediaCheckbox.checked;
      mediaControlsGroup.style.opacity = active ? "1" : "0.5";
      mediaControlsGroup.style.pointerEvents = active ? "auto" : "none";
    }
  }

  if (autoTagNewCheckbox) autoTagNewCheckbox.addEventListener("change", updateDependencyStates);
  if (autoTagUpdatedCheckbox) autoTagUpdatedCheckbox.addEventListener("change", updateDependencyStates);
  if (pruneOldCheckbox) pruneOldCheckbox.addEventListener("change", updateDependencyStates);
  if (enableMediaCheckbox) enableMediaCheckbox.addEventListener("change", updateDependencyStates);

  updateDependencyStates();

  if (form) {
    form.addEventListener("submit", async function (e) {
      e.preventDefault();

      saveBtn.disabled = true;
      saveBtn.innerHTML = '<i class="fa fa-spinner fa-spin mr-1" aria-hidden="true"></i> Saving...';
      statusSpan.textContent = "Saving proposal automation rules...";

      const getVal = (id, def = "") => {
        const el = document.getElementById(id);
        return el ? el.value.trim() : def;
      };
      const getNum = (id, def = 0) => {
        const el = document.getElementById(id);
        return el ? parseInt(el.value, 10) || def : def;
      };
      const getChecked = (id) => {
        const el = document.getElementById(id);
        return el ? el.checked : false;
      };

      const payload = {
        deck: 0,
        strip_colors: getChecked("strip_colors"),
        strip_span: getChecked("strip_span"),
        strip_inline_styles: getChecked("strip_inline_styles"),
        strip_font_tags: getChecked("strip_font_tags"),
        prevent_center_on_back: getChecked("prevent_center_on_back"),
        trim_trailing_breaks: getChecked("trim_trailing_breaks"),
        normalize_spaces: getChecked("normalize_spaces"),
        auto_repair_cloze: getChecked("auto_repair_cloze"),
        strip_empty_elements: getChecked("strip_empty_elements"),

        auto_tag_new_cards: getChecked("auto_tag_new_cards"),
        new_card_tag_pattern: getVal("new_card_tag_pattern"),
        auto_tag_updated_cards: getChecked("auto_tag_updated_cards"),
        updated_card_tag_pattern: getVal("updated_card_tag_pattern"),
        prune_old_update_tags: getChecked("prune_old_update_tags"),
        old_update_tag_pattern: getVal("old_update_tag_pattern"),
        forbidden_tags: getVal("forbidden_tags"),
        normalize_tag_casing: getVal("normalize_tag_casing", "none"),
        protected_tag_namespaces: getVal("protected_tag_namespaces"),

        enable_media_compression: getChecked("enable_media_compression"),
        max_image_dimension: getNum("max_image_dimension", 0),
        image_quality: getNum("image_quality", 0),
        auto_convert_heavy_images: getChecked("auto_convert_heavy_images"),
        strip_exif_metadata: getChecked("strip_exif_metadata"),
        max_media_file_size_mb: getNum("max_media_file_size_mb", 0),

        require_rationale: getChecked("require_rationale"),
        disallow_deletion_suggestions: getChecked("disallow_deletion_suggestions"),
        max_cards_per_proposal: getNum("max_cards_per_proposal", 0),

        custom_html_rules: getVal("custom_html_rules"),
        custom_tag_rules: getVal("custom_tag_rules")
      };

      try {
        const response = await fetch(`/DeckProposalRules/${deckHash}`, {
          method: "POST",
          headers: {
            "Content-Type": "application/json"
          },
          body: JSON.stringify(payload)
        });

        if (response.ok) {
          if (typeof toastr !== "undefined") {
            toastr.success("Proposal automation rules successfully saved.");
          }
          statusSpan.textContent = "All changes saved successfully.";
        } else {
          const errText = await response.text();
          if (typeof toastr !== "undefined") {
            toastr.error("Error saving rules: " + errText);
          }
          statusSpan.textContent = "Error saving rules.";
        }
      } catch (err) {
        if (typeof toastr !== "undefined") {
          toastr.error("Network error while saving rules.");
        }
        statusSpan.textContent = "Network error.";
      } finally {
        saveBtn.disabled = false;
        saveBtn.innerHTML = '<i class="fa fa-check mr-1" aria-hidden="true"></i> Save Automation Rules';
      }
    });
  }

  // ==========================================
  // Interactive Proposal Simulator & Playground
  // ==========================================
  const rawHtmlInput = document.getElementById("playground_raw_html");
  const rawTagsInput = document.getElementById("playground_raw_tags");
  const proposalTypeSelect = document.getElementById("playground_proposal_type");
  const rationaleInput = document.getElementById("playground_rationale");

  const btnVisualMode = document.getElementById("btnPlaygroundModeVisual");
  const btnCodeMode = document.getElementById("btnPlaygroundModeCode");
  const visualContainer = document.getElementById("playground-visual-container");
  const codeContainer = document.getElementById("playground-code-container");
  const richEditor = document.getElementById("playground_rich_editor");
  const codeEditor = document.getElementById("playground_code_editor");
  const cardPreview = document.getElementById("playground_card_preview");
  const tagsPreview = document.getElementById("playground_tags_preview");
  const gateBadge = document.getElementById("playground_gate_badge");

  const btnLoadDirty = document.getElementById("btnLoadDirtySample");
  const btnLoadCloze = document.getElementById("btnLoadClozeSample");
  const btnReset = document.getElementById("btnResetSample");
  const clozeBtn = document.getElementById("btnAnkiCloze");
  const colorPicker = document.getElementById("ankiColorPicker");

  let playgroundMode = "visual"; // 'visual' | 'code'

  const sampleDirtyHtml =
    '<span style="font-family: \'Comic Sans MS\'; font-size: 20px; color: #e74c3c; background-color: #f1c40f;"><b>Heart failure:</b></span><br><br><center>First-line therapy {{c1:ACE inhibitors or ARBs}}</center><br><font color="gray">Non-standard personal note</font><b></b><i></i>&nbsp;&nbsp;&nbsp;<br><br><br>';

  const sampleClozeHtml =
    '<p><span style="color: #2980b9; font-size: 18px;">Beta-blockers</span> indicated for ejection fraction {{c1:reduced &lt; 40%}}.</p><center>Monitor heart rate and blood pressure.</center><br><br>';

  const sampleCleanHtml =
    "<b>Heart failure:</b> First-line therapy {{c1::ACE inhibitors or ARBs}}.<br>Monitor renal function and serum potassium.";

  function getActiveRulesState() {
    const getChk = (id) => (document.getElementById(id) ? document.getElementById(id).checked : false);
    const getTxt = (id, def = "") => (document.getElementById(id) ? document.getElementById(id).value.trim() : def);
    const getInt = (id, def = 0) => {
      const el = document.getElementById(id);
      return el ? parseInt(el.value, 10) || def : def;
    };

    return {
      strip_colors: getChk("strip_colors"),
      strip_span: getChk("strip_span"),
      strip_inline_styles: getChk("strip_inline_styles"),
      strip_font_tags: getChk("strip_font_tags"),
      prevent_center_on_back: getChk("prevent_center_on_back"),
      trim_trailing_breaks: getChk("trim_trailing_breaks"),
      normalize_spaces: getChk("normalize_spaces"),
      auto_repair_cloze: getChk("auto_repair_cloze"),
      strip_empty_elements: getChk("strip_empty_elements"),

      auto_tag_new_cards: getChk("auto_tag_new_cards"),
      new_card_tag_pattern: getTxt("new_card_tag_pattern"),
      auto_tag_updated_cards: getChk("auto_tag_updated_cards"),
      updated_card_tag_pattern: getTxt("updated_card_tag_pattern"),
      prune_old_update_tags: getChk("prune_old_update_tags"),
      old_update_tag_pattern: getTxt("old_update_tag_pattern"),
      forbidden_tags: getTxt("forbidden_tags"),
      normalize_tag_casing: getTxt("normalize_tag_casing", "none"),
      protected_tag_namespaces: getTxt("protected_tag_namespaces"),

      require_rationale: getChk("require_rationale"),
      disallow_deletion_suggestions: getChk("disallow_deletion_suggestions"),
      max_cards_per_proposal: getInt("max_cards_per_proposal", 0),

      custom_html_rules: getTxt("custom_html_rules"),
      custom_tag_rules: getTxt("custom_tag_rules")
    };
  }

  function resolvePattern(pattern) {
    if (!pattern) return "";
    const now = new Date();
    const yyyy = now.getFullYear().toString();
    const yy = yyyy.substring(2);
    const mm = String(now.getMonth() + 1).padStart(2, "0");
    const dd = String(now.getDate()).padStart(2, "0");

    return pattern
      .replace(/\{YYYY\}/g, yyyy)
      .replace(/\{YY\}/g, yy)
      .replace(/\{MM\}/g, mm)
      .replace(/\{DD\}/g, dd)
      .replace(/\{YYYY-MM\}/g, `${yyyy}-${mm}`)
      .replace(/\{MM\/YY\}/g, `${mm}/${yy}`)
      .replace(/\{MM-YY\}/g, `${mm}-${yy}`);
  }

  function matchesWildcard(pattern, tag) {
    const p = pattern.trim();
    if (!p) return false;
    if (p.endsWith("*")) {
      const prefix = p.slice(0, -1);
      return tag.toLowerCase().startsWith(prefix.toLowerCase());
    }
    return tag.toLowerCase() === p.toLowerCase();
  }

  function applyClientCustomHtmlRules(inputHtml, customRules) {
    if (!customRules || !customRules.trim()) return inputHtml;
    let html = inputHtml;

    const lines = customRules.split("\n");
    for (const line of lines) {
      const trimmed = line.trim();
      if (!trimmed || trimmed.startsWith("#")) continue;

      const colonIdx = trimmed.indexOf(":");
      if (colonIdx === -1) continue;

      const directive = trimmed.substring(0, colonIdx).trim().toLowerCase();
      const args = trimmed.substring(colonIdx + 1).trim();

      if (directive === "strip-tags" || directive === "unwrap-tags") {
        const tags = args.split(",").map((s) => s.trim().toLowerCase()).filter(Boolean);
        for (const t of tags) {
          const openRe = new RegExp(`<${t}(\\s+[^>]*)?>`, "gi");
          const closeRe = new RegExp(`</${t}>`, "gi");
          html = html.replace(openRe, "").replace(closeRe, "");
        }
      } else if (directive === "remove-elements") {
        const tags = args.split(",").map((s) => s.trim().toLowerCase()).filter(Boolean);
        for (const t of tags) {
          const elRe = new RegExp(`<${t}(\\s+[^>]*)?>[\\s\\S]*?</${t}>`, "gi");
          html = html.replace(elRe, "");
        }
      } else if (directive === "strip-styles") {
        const props = args.split(",").map((s) => s.trim().toLowerCase()).filter(Boolean);
        for (const p of props) {
          const propRe = new RegExp(`${p}\\s*:\\s*[^;"]+;?`, "gi");
          html = html.replace(/\s+style\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]*)/gi, (full) => {
            const cleaned = full.replace(propRe, "");
            if (cleaned.includes('style=""') || cleaned.includes("style=''") || cleaned.trim() === "style=") {
              return "";
            }
            return cleaned;
          });
        }
      } else if (directive === "strip-attributes") {
        const attrs = args.split(",").map((s) => s.trim().toLowerCase()).filter(Boolean);
        for (const a of attrs) {
          const attrRe = new RegExp(`\\s+${a}\\s*=\\s*(?:"[^"]*"|'[^']*'|[^\\s>]*)`, "gi");
          html = html.replace(attrRe, "");
        }
      } else if (directive === "strip-empty") {
        const tags = args.split(",").map((s) => s.trim().toLowerCase()).filter(Boolean);
        for (const t of tags) {
          const emptyRe = new RegExp(`<${t}(\\s+[^>]*)?>\\s*</${t}>`, "gi");
          for (let k = 0; k < 3; k++) {
            const next = html.replace(emptyRe, "");
            if (next === html) break;
            html = next;
          }
        }
      } else if (directive === "replace") {
        const parts = args.split("=>");
        if (parts.length === 2) {
          const fromStr = parts[0].trim().replace(/^["']|["']$/g, "");
          const toStr = parts[1].trim().replace(/^["']|["']$/g, "");
          if (fromStr) {
            html = html.split(fromStr).join(toStr);
          }
        }
      } else if (directive === "regex-replace") {
        const parts = args.split("=>");
        if (parts.length === 2) {
          let patStr = parts[0].trim();
          if (patStr.startsWith("/") && patStr.endsWith("/")) {
            patStr = patStr.slice(1, -1);
          }
          const toStr = parts[1].trim().replace(/^["']|["']$/g, "");
          try {
            const re = new RegExp(patStr, "g");
            html = html.replace(re, toStr);
          } catch (e) {
            // invalid regex, ignore
          }
        }
      }
    }

    return html;
  }

  function applyClientCustomTagRules(tags, customRules) {
    if (!customRules || !customRules.trim()) return tags;
    let result = [...tags];

    const lines = customRules.split("\n");
    for (const line of lines) {
      const trimmed = line.trim();
      if (!trimmed || trimmed.startsWith("#")) continue;

      const colonIdx = trimmed.indexOf(":");
      if (colonIdx === -1) continue;

      const directive = trimmed.substring(0, colonIdx).trim().toLowerCase();
      const args = trimmed.substring(colonIdx + 1).trim();

      if (directive === "block" || directive === "remove") {
        const patterns = args.split(",").map((s) => s.trim().toLowerCase()).filter(Boolean);
        result = result.filter((t) => {
          const tLow = t.toLowerCase();
          for (const p of patterns) {
            if (p.endsWith("*")) {
              if (tLow.startsWith(p.slice(0, -1))) return false;
            } else if (tLow === p) {
              return false;
            }
          }
          return true;
        });
      } else if (directive === "allow-only") {
        const patterns = args.split(",").map((s) => s.trim().toLowerCase()).filter(Boolean);
        if (patterns.length > 0) {
          result = result.filter((t) => {
            const tLow = t.toLowerCase();
            return patterns.some((p) => {
              if (p.endsWith("*")) {
                return tLow.startsWith(p.slice(0, -1));
              }
              return tLow === p;
            });
          });
        }
      } else if (directive === "replace") {
        const parts = args.split("=>");
        if (parts.length === 2) {
          const fromStr = parts[0].trim().replace(/^["']|["']$/g, "");
          const toStr = parts[1].trim().replace(/^["']|["']$/g, "");
          result = result.map((t) => (t === fromStr ? toStr : t));
        }
      } else if (directive === "prefix-all") {
        const prefix = args.trim().replace(/^["']|["']$/g, "");
        if (prefix) {
          result = result.map((t) => (t.startsWith(prefix) ? t : prefix + t));
        }
      } else if (directive === "case") {
        const c = args.trim().toLowerCase();
        if (c === "lower" || c === "lowercase") {
          result = result.map((t) => t.toLowerCase());
        } else if (c === "upper" || c === "uppercase") {
          result = result.map((t) => t.toUpperCase());
        }
      }
    }

    return result;
  }

  function clientSanitizeHtml(inputHtml, rules) {
    if (!inputHtml) return "";
    let html = inputHtml;

    // 1. Cloze auto-repair
    if (rules.auto_repair_cloze) {
      html = html.replace(/\{\{c(\d+):([^\}:][^\}]*)\}\}/gi, "{{c$1::$2}}");
    }

    // 2. Non-breaking spaces
    if (rules.normalize_spaces) {
      html = html.replace(/&nbsp;/gi, " ");
    }

    // DOM-based cleaning
    const parser = new DOMParser();
    const doc = parser.parseFromString(html, "text/html");
    const body = doc.body;

    // Strict inline styles removal
    if (rules.strip_inline_styles) {
      body.querySelectorAll("*").forEach((el) => el.removeAttribute("style"));
    } else if (rules.strip_colors || rules.prevent_center_on_back) {
      body.querySelectorAll("*").forEach((el) => {
        const style = el.getAttribute("style");
        if (style) {
          const decls = style.split(";").map((d) => d.trim()).filter(Boolean);
          const cleanDecls = decls.filter((decl) => {
            const parts = decl.split(":");
            if (parts.length < 2) return false;
            const prop = parts[0].trim().toLowerCase();
            const val = parts.slice(1).join(":").trim().toLowerCase();
            if (rules.strip_colors && (prop === "color" || prop === "background-color" || prop === "background")) return false;
            if (rules.prevent_center_on_back && prop === "text-align" && val === "center") return false;
            return true;
          });
          if (cleanDecls.length > 0) {
            el.setAttribute("style", cleanDecls.join("; ") + ";");
          } else {
            el.removeAttribute("style");
          }
        }
      });
    }

    // Strip <font> tags or font colors
    if (rules.strip_font_tags) {
      body.querySelectorAll("font").forEach((fontEl) => {
        fontEl.replaceWith(...fontEl.childNodes);
      });
    } else if (rules.strip_colors) {
      body.querySelectorAll("font[color]").forEach((fontEl) => {
        fontEl.removeAttribute("color");
      });
    }

    // Prevent center on verso
    if (rules.prevent_center_on_back) {
      body.querySelectorAll("center").forEach((centerEl) => {
        centerEl.replaceWith(...centerEl.childNodes);
      });
    }

    // Unwrap <span> tags
    if (rules.strip_span) {
      body.querySelectorAll("span").forEach((spanEl) => {
        spanEl.replaceWith(...spanEl.childNodes);
      });
    }

    // Strip empty elements
    if (rules.strip_empty_elements) {
      let removedAny = true;
      let passes = 0;
      while (removedAny && passes < 5) {
        removedAny = false;
        passes++;
        body.querySelectorAll("b, i, u, em, strong, span, font, p, div").forEach((el) => {
          if (!el.hasChildNodes() || el.textContent.trim() === "") {
            el.remove();
            removedAny = true;
          }
        });
      }
    }

    let cleaned = body.innerHTML;

    // Trim trailing linebreaks
    if (rules.trim_trailing_breaks) {
      cleaned = cleaned.replace(/(<br\s*\/?>\s*)+$/gi, "").trim();
    }

    // Apply handwritten custom HTML rules (.config)
    cleaned = applyClientCustomHtmlRules(cleaned, rules.custom_html_rules);

    return cleaned;
  }

  function stripDangerousHtml(html) {
    if (!html) return "";
    const parser = new DOMParser();
    const doc = parser.parseFromString(html, "text/html");
    const blockedTags = ["script", "iframe", "object", "embed", "form", "link", "meta", "style"];
    blockedTags.forEach((tag) => {
      doc.querySelectorAll(tag).forEach((el) => el.remove());
    });
    const allElements = doc.querySelectorAll("*");
    allElements.forEach((el) => {
      for (let i = el.attributes.length - 1; i >= 0; i--) {
        const attr = el.attributes[i];
        const name = attr.name.toLowerCase();
        const val = attr.value.trim().toLowerCase();
        if (name.startsWith("on") || val.startsWith("javascript:")) {
          el.removeAttribute(attr.name);
        }
      }
    });
    return doc.body.innerHTML;
  }

  function updatePlayground() {
    if (!rawHtmlInput) return;
    const rules = getActiveRulesState();
    const rawHtml = rawHtmlInput.value;
    const sanitizedHtml = stripDangerousHtml(clientSanitizeHtml(rawHtml, rules));

    // Update WYSIWYG & Code container
    if (richEditor) richEditor.innerHTML = sanitizedHtml;
    if (codeEditor) codeEditor.textContent = sanitizedHtml;

    // Update Live Flashcard Preview (with Anki cloze rendering)
    let previewContent = sanitizedHtml;
    previewContent = previewContent.replace(/\{\{c(\d+)::([^\}]+)\}\}/g, '<span class="anki-preview-cloze">[$2]</span>');
    cardPreview.innerHTML = previewContent || '<span class="text-muted font-italic">Empty preview</span>';

    // Update Simulated Tags
    updateSimulatedTags(rules);

    // Update PR Quality Gate Evaluation
    updateGateEvaluation(rules);
  }

  function updateSimulatedTags(rules) {
    if (!tagsPreview || !rawTagsInput) return;

    const rawTags = rawTagsInput.value
      .split(",")
      .map((t) => t.trim())
      .filter(Boolean);

    const forbiddenPatterns = rules.forbidden_tags ? rules.forbidden_tags.split(",").map((p) => p.trim()).filter(Boolean) : [];
    const proposalType = proposalTypeSelect ? proposalTypeSelect.value : "update";

    let retainedTags = [];
    const strippedTags = [];

    for (const tag of rawTags) {
      let isBlacklisted = false;
      for (const pattern of forbiddenPatterns) {
        if (matchesWildcard(pattern, tag)) {
          isBlacklisted = true;
          break;
        }
      }

      if (isBlacklisted) {
        strippedTags.push(tag);
      } else {
        let finalTag = tag;
        if (rules.normalize_tag_casing === "lower") {
          finalTag = finalTag.toLowerCase();
        } else if (rules.normalize_tag_casing === "upper") {
          finalTag = finalTag.toUpperCase();
        }
        retainedTags.push(finalTag);
      }
    }

    // Apply custom handwritten tag rules
    retainedTags = applyClientCustomTagRules(retainedTags, rules.custom_tag_rules);

    // Apply auto-tagging
    const autoTags = [];
    if (proposalType === "new" && rules.auto_tag_new_cards) {
      const newTag = resolvePattern(rules.new_card_tag_pattern);
      if (newTag) autoTags.push(newTag);
    } else if (proposalType === "update" && rules.auto_tag_updated_cards) {
      const updateTag = resolvePattern(rules.updated_card_tag_pattern);
      if (updateTag) autoTags.push(updateTag);

      // Prune old update tags if enabled
      if (rules.prune_old_update_tags && rules.old_update_tag_pattern) {
        const prunePattern = rules.old_update_tag_pattern;
        for (let i = retainedTags.length - 1; i >= 0; i--) {
          if (matchesWildcard(prunePattern, retainedTags[i])) {
            strippedTags.push(retainedTags[i] + " (pruned)");
            retainedTags.splice(i, 1);
          }
        }
      }
    }

    // Render HTML badges (escaping tag strings to prevent DOM XSS)
    const escapeHtml = (str) =>
      String(str)
        .replace(/&/g, "&amp;")
        .replace(/</g, "&lt;")
        .replace(/>/g, "&gt;")
        .replace(/"/g, "&quot;")
        .replace(/'/g, "&#39;");

    let badgesHtml = "";
    autoTags.forEach((tag) => {
      badgesHtml += `<span class="badge-tag-auto" title="Automatically generated tag">+ ${escapeHtml(tag)}</span>`;
    });
    retainedTags.forEach((tag) => {
      badgesHtml += `<span class="badge-tag-kept">${escapeHtml(tag)}</span>`;
    });
    strippedTags.forEach((tag) => {
      badgesHtml += `<span class="badge-tag-stripped" title="Stripped by blacklist or version pruning">${escapeHtml(tag)}</span>`;
    });

    if (!badgesHtml) {
      badgesHtml = '<span class="text-muted small">No tags applied</span>';
    }
    tagsPreview.innerHTML = badgesHtml;
  }

  function updateGateEvaluation(rules) {
    if (!gateBadge) return;
    const proposalType = proposalTypeSelect ? proposalTypeSelect.value : "update";
    const rationale = rationaleInput ? rationaleInput.value.trim() : "";

    const violations = [];
    if (rules.require_rationale && !rationale) {
      violations.push("Rejected: Rationale / explanation comment required");
    }
    if (rules.disallow_deletion_suggestions && proposalType === "deletion") {
      violations.push("Rejected: Card deletions restricted to deck maintainers");
    }

    if (violations.length === 0) {
      gateBadge.className = "badge badge-success font-weight-normal px-2 py-1";
      gateBadge.textContent = "All PR quality gates pass";
    } else {
      gateBadge.className = "badge badge-danger font-weight-normal px-2 py-1";
      gateBadge.textContent = violations[0];
    }
  }

  // Preset Sample Listeners
  if (btnLoadDirty) {
    btnLoadDirty.addEventListener("click", function () {
      rawHtmlInput.value = sampleDirtyHtml;
      rawTagsInput.value = "Cardiology, Topic-232, marked, leech, flag1, HighYield, UPDATED::05/25";
      rationaleInput.value = "Updated dosage guidelines and added explanatory color formatting.";
      proposalTypeSelect.value = "update";
      updatePlayground();
    });
  }

  if (btnLoadCloze) {
    btnLoadCloze.addEventListener("click", function () {
      rawHtmlInput.value = sampleClozeHtml;
      rawTagsInput.value = "Cardiology, HighYield, todo, personal";
      rationaleInput.value = "Added ejection fraction cutoff threshold.";
      proposalTypeSelect.value = "new";
      updatePlayground();
    });
  }

  if (btnReset) {
    btnReset.addEventListener("click", function () {
      rawHtmlInput.value = sampleCleanHtml;
      rawTagsInput.value = "Cardiology, Topic-232, HighYield";
      rationaleInput.value = "Standard maintenance update.";
      proposalTypeSelect.value = "update";
      updatePlayground();
    });
  }

  // Mode Switcher Listeners
  if (btnVisualMode) {
    btnVisualMode.addEventListener("click", function () {
      if (playgroundMode === "code") {
        playgroundMode = "visual";
        btnVisualMode.classList.add("active");
        btnCodeMode.classList.remove("active");
        visualContainer.style.display = "block";
        codeContainer.style.display = "none";
        richEditor.innerHTML = stripDangerousHtml(codeEditor.textContent);
      }
    });
  }

  if (btnCodeMode) {
    btnCodeMode.addEventListener("click", function () {
      if (playgroundMode === "visual") {
        playgroundMode = "code";
        btnCodeMode.classList.add("active");
        btnVisualMode.classList.remove("active");
        visualContainer.style.display = "none";
        codeContainer.style.display = "block";
        codeEditor.textContent = richEditor.innerHTML;
      }
    });
  }

  // Toolbar Formatting Handlers
  document.querySelectorAll(".anki-toolbar .anki-btn[data-cmd]").forEach((btn) => {
    btn.addEventListener("click", function () {
      const cmd = this.getAttribute("data-cmd");
      document.execCommand(cmd, false, null);
      richEditor.focus();
      cardPreview.innerHTML = stripDangerousHtml(richEditor.innerHTML);
    });
  });

  if (colorPicker) {
    colorPicker.addEventListener("change", function () {
      document.execCommand("foreColor", false, this.value);
      richEditor.focus();
      cardPreview.innerHTML = stripDangerousHtml(richEditor.innerHTML);
    });
  }

  if (clozeBtn) {
    clozeBtn.addEventListener("click", function () {
      const selection = window.getSelection();
      const selectedText = selection.toString() || "word";
      document.execCommand("insertText", false, `{{c1::${selectedText}}}`);
      richEditor.focus();
      cardPreview.innerHTML = stripDangerousHtml(richEditor.innerHTML).replace(/\{\{c(\d+)::([^\}]+)\}\}/g, '<span class="anki-preview-cloze">[$2]</span>');
    });
  }

  // Reactivity Listeners for All Form Controls
  if (rawHtmlInput) rawHtmlInput.addEventListener("input", updatePlayground);
  if (rawTagsInput) rawTagsInput.addEventListener("input", updatePlayground);
  if (proposalTypeSelect) proposalTypeSelect.addEventListener("change", updatePlayground);
  if (rationaleInput) rationaleInput.addEventListener("input", updatePlayground);

  // Listen on all rule switches and text inputs to immediately re-sanitize the playground
  const ruleSelectors = [
    "#strip_colors",
    "#strip_span",
    "#strip_inline_styles",
    "#strip_font_tags",
    "#prevent_center_on_back",
    "#trim_trailing_breaks",
    "#normalize_spaces",
    "#auto_repair_cloze",
    "#strip_empty_elements",
    "#auto_tag_new_cards",
    "#new_card_tag_pattern",
    "#auto_tag_updated_cards",
    "#updated_card_tag_pattern",
    "#prune_old_update_tags",
    "#old_update_tag_pattern",
    "#forbidden_tags",
    "#normalize_tag_casing",
    "#protected_tag_namespaces",
    "#require_rationale",
    "#disallow_deletion_suggestions",
    "#max_cards_per_proposal",
    "#custom_html_rules",
    "#custom_tag_rules"
  ];

  ruleSelectors.forEach((sel) => {
    const el = document.querySelector(sel);
    if (el) {
      el.addEventListener("change", updatePlayground);
      el.addEventListener("input", updatePlayground);
    }
  });

  // Initial load
  if (rawHtmlInput && !rawHtmlInput.value) {
    rawHtmlInput.value = sampleDirtyHtml;
  }
  updatePlayground();
});
