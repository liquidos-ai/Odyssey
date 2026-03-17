use crate::skill::BundleSkillStore;

pub(crate) fn build_system_prompt(
    base_prompt: &str,
    skills: &BundleSkillStore,
    include_skills: bool,
) -> String {
    if !include_skills {
        return base_prompt.to_string();
    }
    format!(
        "{}\n\n---\n\n{}",
        base_prompt.trim_end(),
        skills.render_prompt_section()
    )
}

#[cfg(test)]
mod tests {
    use super::build_system_prompt;
    use crate::skill::BundleSkillStore;
    use pretty_assertions::assert_eq;
    use std::fs;

    #[test]
    fn appends_skill_section_when_enabled() {
        let temp = tempfile::tempdir().expect("tempdir");
        let skill_dir = temp.path().join("skills").join("review");
        fs::create_dir_all(&skill_dir).expect("mkdir");
        fs::write(
            skill_dir.join("SKILL.md"),
            "# Review\n\nHelps review code changes.\n",
        )
        .expect("write");
        let skills = BundleSkillStore::load(temp.path()).expect("skills");

        let prompt = build_system_prompt("Base prompt", &skills, true);

        assert!(prompt.contains("Base prompt"));
        assert!(prompt.contains("## Skills"));
        assert!(prompt.contains("- review: Helps review code changes."));
    }

    #[test]
    fn leaves_prompt_unchanged_when_skill_section_disabled() {
        let skills = BundleSkillStore::default();
        let prompt = build_system_prompt("Base prompt", &skills, false);
        assert_eq!(prompt, "Base prompt");
    }
}
