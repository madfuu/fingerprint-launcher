//! `text_enum!` declares a fieldless enum that crosses every boundary as one fixed string per
//! variant: serde (bridge DTOs), SQLite TEXT columns and `FromStr` all use the same literal, so
//! the three representations cannot drift apart.
//!
//! Variant order is significant: the derived `Ord` follows it, and callers use it as the
//! canonical order (e.g. the `--disable-spoofing` CSV).

macro_rules! text_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $( $(#[$vmeta:meta])* $variant:ident => $text:literal ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $name {
            $( $(#[$vmeta])* $variant ),+
        }

        impl $name {
            /// The literal used on the wire and in the database.
            pub fn as_str(self) -> &'static str {
                match self {
                    $( Self::$variant => $text ),+
                }
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl ::std::str::FromStr for $name {
            type Err = $crate::error::AppError;

            fn from_str(s: &str) -> ::std::result::Result<Self, Self::Err> {
                match s {
                    $( $text => Ok(Self::$variant), )+
                    _ => Err($crate::error::AppError::invalid_input(format!(
                        concat!("unknown ", stringify!($name), " value {:?}"),
                        s
                    ))),
                }
            }
        }

        impl ::serde::Serialize for $name {
            fn serialize<S: ::serde::Serializer>(
                &self,
                serializer: S,
            ) -> ::std::result::Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D: ::serde::Deserializer<'de>>(
                deserializer: D,
            ) -> ::std::result::Result<Self, D::Error> {
                let text = <String as ::serde::Deserialize>::deserialize(deserializer)?;
                text.parse()
                    .map_err(|_| ::serde::de::Error::unknown_variant(&text, &[$($text),+]))
            }
        }

        impl ::rusqlite::types::ToSql for $name {
            fn to_sql(&self) -> ::rusqlite::Result<::rusqlite::types::ToSqlOutput<'_>> {
                Ok(self.as_str().into())
            }
        }

        impl ::rusqlite::types::FromSql for $name {
            fn column_result(
                value: ::rusqlite::types::ValueRef<'_>,
            ) -> ::rusqlite::types::FromSqlResult<Self> {
                value
                    .as_str()?
                    .parse()
                    .map_err(::rusqlite::types::FromSqlError::other)
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    text_enum! {
        /// Test enum.
        pub enum Fruit {
            Apple => "apple",
            BloodOrange => "blood-orange",
        }
    }

    #[test]
    fn same_literal_for_serde_sql_and_from_str() {
        assert_eq!(
            serde_json::to_string(&Fruit::BloodOrange).unwrap(),
            "\"blood-orange\""
        );
        assert_eq!(
            serde_json::from_str::<Fruit>("\"apple\"").unwrap(),
            Fruit::Apple
        );
        let err = serde_json::from_str::<Fruit>("\"pear\"").unwrap_err();
        assert!(err.to_string().contains("unknown variant `pear`"), "{err}");
        assert_eq!("blood-orange".parse::<Fruit>().unwrap(), Fruit::BloodOrange);
        assert!("Apple".parse::<Fruit>().is_err());
        assert!(Fruit::Apple < Fruit::BloodOrange);

        let conn = Connection::open_in_memory().unwrap();
        let stored: String = conn
            .query_row("SELECT ?1", [Fruit::BloodOrange], |r| r.get(0))
            .unwrap();
        assert_eq!(stored, "blood-orange");
        let read: Fruit = conn.query_row("SELECT 'apple'", [], |r| r.get(0)).unwrap();
        assert_eq!(read, Fruit::Apple);
        assert!(
            conn.query_row("SELECT 'pear'", [], |r| r.get::<_, Fruit>(0))
                .is_err()
        );
    }
}
