use serde::{Deserialize, Deserializer};

#[derive(Debug)]
pub enum OneOrMore<T> {
    Zero,
    One(T),
    More,
}

impl<T> OneOrMore<T> {
    pub fn extend(self, other: Self) -> Self {
        match (self, other) {
            (OneOrMore::Zero, OneOrMore::Zero) => OneOrMore::Zero,
            (OneOrMore::One(x), OneOrMore::Zero) | (OneOrMore::Zero, OneOrMore::One(x)) => {
                OneOrMore::One(x)
            }
            _ => OneOrMore::More,
        }
    }
}

impl<'de, T> Deserialize<'de> for OneOrMore<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let vec = Vec::deserialize(deserializer)?;

        Ok(match &*vec {
            [] => Self::Zero,
            [_one] => {
                let [one] = <[T; 1]>::try_from(vec).ok().unwrap();
                Self::One(one)
            }
            _ => Self::More,
        })
    }
}
