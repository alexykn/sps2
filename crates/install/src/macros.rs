//! Macros for context builder helpers

#[macro_export]
macro_rules! context_add_package_method {
    ($name:ident, $pkg_type:ty) => {
        impl $name {
            /// Add package to the context
            #[must_use]
            pub fn add_package(mut self, package: $pkg_type) -> Self {
                self.packages.push(package);
                self
            }
        }
    };
}

#[macro_export]
macro_rules! context_builder {
    ($name:ident { $($field:ident: $ty:ty),* $(,)? }) => {
        paste::paste! {
            impl $name {
                /// Create a new context with default values
                pub fn new() -> Self {
                    Self {
                        $($field: Default::default(),)*
                        event_sender: None,
                    }
                }

                $( #[must_use]
                pub fn [<with_ $field>](mut self, value: $ty) -> Self {
                    self.$field = value;
                    self
                } )*

                /// Set the event sender for progress reporting
                #[must_use]
                pub fn with_event_sender(mut self, sender: sps2_events::EventSender) -> Self {
                    self.event_sender = Some(sender);
                    self
                }
            }

            impl Default for $name {
                fn default() -> Self {
                    Self::new()
                }
            }
        }
    };
}
