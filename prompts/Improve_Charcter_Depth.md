When enriching a JSON character profile, deepen each field with specific, humanising detail while preserving the original JSON structure and keys. Avoid clichés, generic personality labels, or shallow traits. Expand the character using the following principles:

1. **Origin & Environment**
   - Make the character’s region feel lived-in through sensory detail, local culture, work rhythms, weather, and social norms.
   - Show how the environment shaped their temperament, habits, and emotional tone.

2. **Formative Experiences**
   - Add 2–3 concrete early-life experiences that influenced their worldview, strengths, flaws, and interpersonal style.
   - Tie these experiences to the region, era, or community they grew up in.

3. **Contradictions & Internal Tensions**
   - Introduce subtle contradictions (e.g., calm but stubborn, warm but blunt).
   - Let these tensions appear naturally in their voice, mannerisms, or decision-making.

4. **Voice & Speech Style**
   - Describe cadence, rhythm, vocabulary, emotional coloration, and conversational habits.
   - Ensure the voice feels distinct and consistent with the character’s background.

5. **Mannerisms**
   - Add small, specific behavioural details that reveal personality.
   - Avoid generic gestures; use mannerisms that reflect culture, work, climate, or upbringing.

6. **Cultural & Historical Texture**
   - Anchor the character lightly in cultural references (local trades, community rituals, regional humour, generational attitudes).
   - Use these influences to add depth without overwhelming the profile.

7. **Psychological Core**
   - Identify core motivations, fears, values, and blind spots.
   - Show how these shape their behaviour and interactions.

8. **Social Context & Relationships**
   - Describe how the character tends to interact with others: who they trust, who they clash with, and how they behave in groups vs. one-on-one.

9. **Backstory Construction**
   - Write a backstory that feels like a life lived, not a résumé.
   - Use vivid, specific imagery tied to place, work, weather, and community.
   - Maintain the character’s voice throughout the narrative.

10. **Backstory Lines**
   - Provide 3–5 short, memorable lines that capture the character’s essence.
   - Use varied imagery and avoid repeating metaphors or phrasing.

11. **Avoid Repetition**
   - Keep the character’s tone consistent but vary imagery, sentence structure, and emotional expressions.
   - Ensure each field adds new information rather than rephrasing earlier details.

12. **Preserve JSON Structure**
   - Keep all original keys (e.g., region, origin_detail, voice_style, mannerisms, backstory, backstory_lines).
   - Expand or rewrite values with richer detail while maintaining valid JSON formatting.
   - Write the keys in this exact order, and add no key that is not in it:
     `name`, `voice`, `pitch`, `sex`, `region`, `born`, `job`, `origin_detail`,
     `voice_style`, `political_temperament`, `mannerisms`, `speech_register`,
     `backstory_lines`, `backstory`.
     `speech_register` is the one optional member — keep it where it is if the entry
     already has it, and do not invent one.
   - The file holds `{ "characters": [ ... ] }` and nothing else. Add no other
     top-level key: the country is the filename, so a `country` key would contradict it.
   - Leave the array order exactly as it is. Position is load-bearing: a country that
     ships two characters of the same name relies on the first occurrence keeping the
     bare name.

Use these principles to transform simple character entries into layered, textured, emotionally real personas while keeping the JSON schema intact.
