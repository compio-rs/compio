use std::{
    borrow::{Borrow, Cow},
    hash::{Hash, Hasher},
    sync::Arc,
};

#[derive(Clone)]
pub(crate) enum Name {
    Static(&'static str),
    Owned(Arc<str>),
}

impl Name {
    pub(crate) fn as_str(&self) -> &str {
        match self {
            Self::Static(name) => name,
            Self::Owned(name) => name,
        }
    }

    pub(crate) fn into_cow(self) -> Cow<'static, str> {
        match self {
            Self::Static(name) => Cow::Borrowed(name),
            Self::Owned(name) => Cow::Owned(name.as_ref().to_owned()),
        }
    }
}

impl From<Cow<'static, str>> for Name {
    fn from(name: Cow<'static, str>) -> Self {
        match name {
            Cow::Borrowed(name) => Self::Static(name),
            Cow::Owned(name) => Self::Owned(Arc::from(name)),
        }
    }
}

impl Borrow<str> for Name {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq for Name {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for Name {}

impl Hash for Name {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}
